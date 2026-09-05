//! Gerrit backend: `ForgeBackend` over the Gerrit REST API.
//!
//! Unlike the other forges, Gerrit ships no companion CLI, so there is nothing
//! to shell out to. The transport is HTTPS via [`ureq`], with the same
//! mockable-runner shape the other backends use ([`GerritHttp`]) so the
//! `ForgeBackend` impl is testable without a server.
//!
//! Auth is Gerrit's HTTP password (Settings → HTTP Credentials), sent as HTTP
//! Basic. Authenticated calls go through Gerrit's `/a/` prefix; without
//! credentials the backend still reads public changes anonymously, but
//! submitting a review needs them.
//!
//! Diffs come from a local clone (`git diff <base>..<head>` after fetching the
//! change's patch-set ref) rather than from `/revisions/{id}/patch`. Gerrit's
//! file list is keyed and sorted by path while its patch output is ordered by
//! git's rename-aware diff queue, so the two cannot be paired positionally the
//! way `pair_metadata_with_patch` requires — and `git diff --raw` gives us
//! authoritative metadata for free. Azure DevOps takes the same local-clone
//! route for the same reason.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Map, Value, json};

use crate::error::{Result, TuicrError};
use crate::forge::remote_comments::RemoteReviewThread;
use crate::forge::submit::{GhSide, InlineComment, SubmitEvent};
use crate::forge::traits::{
    CreateReviewRequest, ForgeBackend, ForgeFileLinesRequest, ForgeRepository,
    GhCreateReviewResponse, PagedPullRequests, PullRequestCommit, PullRequestDetails,
    PullRequestListQuery, PullRequestListScope, PullRequestTarget,
};
use crate::model::{DiffLine, FilePatch};
use crate::process::run_command_output;
use crate::vcs::git::raw::run_git_diff;
use crate::vcs::slice_context_lines;

use super::models::{GerritChange, GerritComment, threads_from_comment_map};

/// Gerrit's canonical SSH port. A remote on this port is a Gerrit remote, full
/// stop — it is the strongest signal available for self-hosted instances.
const GERRIT_SSH_PORT: &str = "29418";
/// Base URL of the Gerrit server, e.g. `https://review.example.com/gerrit`.
/// Set it when the hostname does not contain "gerrit", when the web host
/// differs from the git remote's host, or when Gerrit is served under a path
/// prefix or a non-default port. When set it overrides the host inferred from
/// the remote — see [`web_base`].
const URL_ENV_VAR: &str = "GERRIT_URL";
/// Gerrit account name for HTTP Basic auth.
const USER_ENV_VAR: &str = "GERRIT_USERNAME";
/// Gerrit HTTP password (Settings → HTTP Credentials), not the account
/// password.
const PASSWORD_ENV_VAR: &str = "GERRIT_PASSWORD";
/// Label a review vote is cast on. `Code-Review` is Gerrit's out-of-the-box
/// review label and is present on every default install.
const REVIEW_LABEL: &str = "Code-Review";
/// How much of a server error body to keep, in characters. Long enough for a
/// real Gerrit message, short enough not to flood the status line.
const MAX_DETAIL_CHARS: usize = 400;
/// Gerrit's magic path for patch-set-level (review-level) comments.
const PATCHSET_LEVEL: &str = "/PATCHSET_LEVEL";
/// Gerrit prefixes every JSON response with this XSSI guard.
const MAGIC_PREFIX: &str = ")]}'";

// ---------- Transport ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GerritHttpError {
    /// 401/403 — authentication or authorization failure.
    Auth(String),
    /// Any other non-2xx status or transport error.
    Failed { status: Option<u16>, body: String },
}

pub type GerritHttpResult<T> = std::result::Result<T, GerritHttpError>;

/// HTTP transport for the Gerrit REST API. Returns the raw 2xx response body,
/// XSSI prefix included — stripping is the caller's job so non-JSON endpoints
/// (file content) pass through untouched.
pub trait GerritHttp: Send + Sync {
    fn request(&self, method: &str, url: &str, body: Option<&str>) -> GerritHttpResult<String>;
    /// Whether this transport carries credentials. Drives the `/a/` prefix and
    /// gates the queries that only mean something for a logged-in user.
    fn is_authenticated(&self) -> bool;
}

/// Direct REST transport, optionally with HTTP Basic credentials.
pub struct HttpsGerrit {
    auth_header: Option<String>,
    agent: ureq::Agent,
}

impl HttpsGerrit {
    pub fn new(credentials: Option<(String, String)>) -> Self {
        let auth_header = credentials.map(|(user, password)| {
            format!("Basic {}", BASE64.encode(format!("{user}:{password}")))
        });
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build();
        Self {
            auth_header,
            agent: config.into(),
        }
    }

    /// Attach HTTP Basic credentials, when there are any. Gerrit reads public
    /// changes anonymously, so an unauthenticated request is not an error.
    fn authorized<B>(&self, builder: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        match self.auth_header.as_deref() {
            Some(auth) => builder.header("Authorization", auth),
            None => builder,
        }
    }
}

impl GerritHttp for HttpsGerrit {
    fn is_authenticated(&self) -> bool {
        self.auth_header.is_some()
    }

    fn request(&self, method: &str, url: &str, body: Option<&str>) -> GerritHttpResult<String> {
        // `ureq` gives with-body and without-body builders different types, so
        // the body-carrying methods share one arm and GET gets its own.
        let result = match method.to_ascii_uppercase().as_str() {
            "GET" => self.authorized(self.agent.get(url)).call(),
            verb @ ("POST" | "PUT") => {
                let builder = match verb {
                    "POST" => self.agent.post(url),
                    _ => self.agent.put(url),
                };
                self.authorized(builder)
                    .header("Content-Type", "application/json")
                    .send(body.unwrap_or(""))
            }
            other => {
                return Err(GerritHttpError::Failed {
                    status: None,
                    body: format!("unsupported HTTP method {other}"),
                });
            }
        };

        let response = result.map_err(|err| GerritHttpError::Failed {
            status: None,
            body: err.to_string(),
        })?;
        let status = response.status().as_u16();
        let text =
            response
                .into_body()
                .read_to_string()
                .map_err(|err| GerritHttpError::Failed {
                    status: Some(status),
                    body: err.to_string(),
                })?;

        if (200..300).contains(&status) {
            Ok(text)
        } else if status == 401 || status == 403 {
            Err(GerritHttpError::Auth(text))
        } else {
            Err(GerritHttpError::Failed {
                status: Some(status),
                body: text,
            })
        }
    }
}

/// Credentials from the environment, when both halves are set.
fn credentials_from_env() -> Option<(String, String)> {
    let user = non_empty_env(USER_ENV_VAR)?;
    let password = non_empty_env(PASSWORD_ENV_VAR)?;
    Some((user, password))
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Strip Gerrit's XSSI guard (`)]}'`) from a JSON response body.
fn strip_magic_prefix(body: &str) -> &str {
    body.strip_prefix(MAGIC_PREFIX)
        .map(|rest| rest.trim_start_matches(['\n', '\r']))
        .unwrap_or(body)
}

// ---------- Coordinate helpers ----------

/// Reverse [`ForgeRepository::gerrit`]'s packing back into a project path.
///
/// `owner` holds the project's parent path, or the host when the project is a
/// single segment (an empty owner would break PR slugs).
pub fn gerrit_project(repo: &ForgeRepository) -> String {
    if repo.owner == repo.host || repo.owner.is_empty() {
        repo.name.clone()
    } else {
        format!("{}/{}", repo.owner, repo.name)
    }
}

/// The configured `GERRIT_URL`, normalized to something an HTTP client will
/// accept: no trailing slash, and always an explicit `http(s)` scheme.
///
/// An `http(s)` URL is kept exactly as written — it may carry a port and a
/// path prefix that matter. Anything else is a bare host or an SSH URL pasted
/// out of `git remote -v`; those are normalized onto HTTPS, because the REST
/// API never speaks SSH and a schemeless value is rejected outright (`http:
/// invalid format`). Userinfo is dropped there, and so is a `29418` port —
/// that is Gerrit's *SSH* port and means nothing over HTTPS. Any other port is
/// kept: someone who writes `review.internal:8443` meant that port, and
/// silently discarding it would be worse than the schemeless case this
/// normalization exists to fix.
fn configured_base_url() -> Option<String> {
    let raw = non_empty_env(URL_ENV_VAR)?;
    let trimmed = raw.trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Some(trimmed.to_string());
    }
    let rest = strip_scheme_and_userinfo(trimmed);
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let authority = match authority.rsplit_once(':') {
        Some((host, GERRIT_SSH_PORT)) => host,
        _ => authority,
    };
    if authority.is_empty() {
        return None;
    }
    let path = path.trim_matches('/');
    if path.is_empty() {
        Some(format!("https://{authority}"))
    } else {
        Some(format!("https://{authority}/{path}"))
    }
}

/// Hostname of `GERRIT_URL`, when set.
fn configured_host() -> Option<String> {
    let url = configured_base_url()?;
    let authority = strip_scheme_and_userinfo(&url).split('/').next()?;
    Some(split_host_port(authority).0.to_ascii_lowercase())
}

/// The path prefix `GERRIT_URL` carries, when Gerrit is served under a
/// subdirectory (`https://review.internal/gerrit` → `gerrit`).
fn configured_path_prefix() -> Option<String> {
    let base = configured_base_url()?;
    let (_, prefix) = strip_scheme_and_userinfo(&base).split_once('/')?;
    let prefix = prefix.trim_matches('/');
    (!prefix.is_empty()).then(|| prefix.to_string())
}

/// The server root every REST and web URL hangs off.
///
/// `GERRIT_URL` wins whenever it is set. It is the override for deployments a
/// git remote cannot describe: a Gerrit whose web host differs from the SSH
/// gateway in the remote (`ssh://gerrit-ssh.corp.com:29418/proj` served at
/// `https://review.corp.com`), a non-default port, or a path prefix. Requiring
/// it to *match* `repo.host` — as this first did — made exactly the split-host
/// case it exists for silently unfixable.
///
/// Otherwise the remote's own host is assumed to serve Gerrit over HTTPS at
/// `/`, which is the stock layout and needs no configuration.
///
/// The variable is process-global, so it names *one* server. Reviewing on two
/// Gerrits means scoping it per checkout (direnv, a shell wrapper) or leaving
/// it unset where the hostnames already carry the signal.
fn web_base(repo: &ForgeRepository) -> String {
    configured_base_url().unwrap_or_else(|| format!("https://{}", repo.host))
}

// ---------- Backend ----------

pub struct GerritBackend {
    default_repository: Option<ForgeRepository>,
    http: Box<dyn GerritHttp>,
    local_checkout: Option<PathBuf>,
}

impl GerritBackend {
    /// Build a backend, reading HTTP credentials from the environment.
    pub fn new(default_repository: Option<ForgeRepository>) -> Self {
        Self {
            default_repository,
            http: Box::new(HttpsGerrit::new(credentials_from_env())),
            local_checkout: None,
        }
    }

    /// Build a backend with an explicit transport (used in tests).
    pub fn with_transport(
        default_repository: Option<ForgeRepository>,
        http: Box<dyn GerritHttp>,
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
                    "Gerrit change target `{}` does not include a project",
                    target.original
                ))
            })
    }

    /// Build an API URL. `path` starts with `/` and is appended to the server
    /// root, behind `/a` when the transport carries credentials.
    fn url(&self, repo: &ForgeRepository, path: &str) -> String {
        let prefix = if self.http.is_authenticated() {
            "/a"
        } else {
            ""
        };
        format!("{}{prefix}{path}", web_base(repo))
    }

    fn get(&self, repo: &ForgeRepository, path: &str) -> Result<String> {
        self.http
            .request("GET", &self.url(repo, path), None)
            .map_err(|err| map_http_error(err, &web_base(repo)))
    }

    fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        repo: &ForgeRepository,
        path: &str,
    ) -> Result<T> {
        let output = self.get(repo, path)?;
        Ok(serde_json::from_str(strip_magic_prefix(&output))?)
    }

    fn send(&self, repo: &ForgeRepository, method: &str, path: &str, body: &str) -> Result<String> {
        self.http
            .request(method, &self.url(repo, path), Some(body))
            .map_err(|err| map_http_error(err, &web_base(repo)))
    }

    /// Fetch one change with everything the detail view needs.
    fn fetch_change(&self, repo: &ForgeRepository, number: u64) -> Result<GerritChange> {
        self.get_json(
            repo,
            &format!("/changes/{number}/?o=CURRENT_REVISION&o=CURRENT_COMMIT&o=DETAILED_ACCOUNTS"),
        )
    }

    /// Ensure both ends of the change's diff exist in `root`, fetching the
    /// patch-set ref when they don't. Gerrit changes live under
    /// `refs/changes/*`, which a normal clone does not carry.
    fn ensure_revision_local(&self, root: &Path, pr: &PullRequestDetails) -> bool {
        if sha_present(root, &pr.base_sha) && sha_present(root, &pr.head_sha) {
            return true;
        }
        let Some(remote) = remote_for_repository(root, &pr.repository) else {
            return false;
        };
        // A bare `git fetch <remote> <ref>` writes FETCH_HEAD only — it creates
        // no branch and moves no existing ref in the user's repo.
        let _ = run_command_output(
            "git",
            Some(root),
            [
                "fetch",
                "--quiet",
                remote.as_str(),
                pr.head_ref_name.as_str(),
            ],
        );
        sha_present(root, &pr.base_sha) && sha_present(root, &pr.head_sha)
    }

    /// File content at the request's revision: local blob first, REST fallback.
    fn file_content(&self, request: &ForgeFileLinesRequest) -> Result<String> {
        if let Some(content) = self
            .local_checkout
            .as_deref()
            .and_then(|root| read_blob(root, request.sha(), request.path.as_path()))
        {
            return Ok(content);
        }
        // The project/commit endpoint, not the change/revision one: a
        // `ForgeFileLinesRequest` carries SHAs but no change number, and this
        // shape addresses either side of the diff by its commit directly.
        let path = format!(
            "/projects/{}/commits/{}/files/{}/content",
            encode_path(&gerrit_project(&request.repository)),
            request.sha(),
            encode_path(&request.path.to_string_lossy()),
        );
        let encoded = self.get(&request.repository, &path)?;
        let bytes = BASE64.decode(encoded.trim()).map_err(|err| {
            TuicrError::Forge(format!("Gerrit returned unreadable content: {err}"))
        })?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// `git diff a..b` in the local clone, after making sure both commits are
    /// present.
    fn local_diff(&self, pr: &PullRequestDetails, a: &str, b: &str) -> Result<Vec<FilePatch>> {
        let root = self
            .local_checkout
            .as_deref()
            .ok_or_else(missing_checkout)?;
        if !self.ensure_revision_local(root, pr) {
            return Err(TuicrError::Forge(format!(
                "Could not find {}..{} in the local checkout. Fetch the change's patch set \
                 (`git fetch origin {}`) and retry.",
                short(a),
                short(b),
                pr.head_ref_name,
            )));
        }
        run_git_diff(root, &[format!("{a}..{b}").as_str()])
    }

    /// Post every inline comment, plus the review body, as *draft* comments.
    /// Gerrit drafts are per-comment rather than per-review, and the body has
    /// a home of its own: the `/PATCHSET_LEVEL` magic path.
    fn create_drafts(
        &self,
        pr: &PullRequestDetails,
        request: &CreateReviewRequest<'_>,
    ) -> Result<()> {
        let path = format!("/changes/{}/revisions/{}/drafts", pr.number, pr.head_sha);
        let review_body = (!request.body.is_empty())
            .then(|| json!({ "path": PATCHSET_LEVEL, "message": request.body }));
        let inline = request.comments.iter().map(|comment| {
            let mut payload = comment_input(comment);
            payload.insert("path".to_string(), json!(comment_path_key(comment)));
            Value::Object(payload)
        });
        for payload in review_body.into_iter().chain(inline) {
            self.send(
                &pr.repository,
                "PUT",
                &path,
                &serde_json::to_string(&payload)?,
            )?;
        }
        Ok(())
    }
}

impl ForgeBackend for GerritBackend {
    fn list_pull_requests(&self, query: PullRequestListQuery) -> Result<PagedPullRequests> {
        let page_size = query.page_size.max(1);
        let project = gerrit_project(&query.repository);
        let mut terms = format!("status:open+project:{}", encode_query_term(&project));
        // The attention set, not `reviewer:self`. `reviewer:self` matches every
        // open change you are a reviewer on, including the ones you already
        // voted on and handed back to the author — which is the opposite of
        // "needs my review". The attention set is Gerrit's own "it's your turn"
        // signal (what the dashboard's *Your Turn* section renders), so it is
        // the only query that makes this toggle mean anything here.
        //
        // `-owner:self` because the attention set also holds *your* changes
        // that a reviewer just replied to — your turn to answer as the author,
        // not to review. Needs Gerrit 3.3+, which introduced the attention set;
        // older servers reject the operator rather than silently mis-filtering.
        //
        // Both operators need a logged-in user, so an anonymous session falls
        // back to every open change rather than erroring.
        if query.scope == PullRequestListScope::ReviewRequested && self.http.is_authenticated() {
            terms.push_str("+attention:self+-owner:self");
        }
        // Fetch one extra to detect a further page, the same probe the Azure
        // backend uses. Gerrit's own `_more_changes` flag would also answer
        // this, but only by riding on the last element — an empty page carries
        // no element, and so no answer.
        let path = format!(
            "/changes/?q={terms}&n={}&S={}&o=CURRENT_REVISION&o=DETAILED_ACCOUNTS",
            page_size + 1,
            query.already_loaded,
        );
        let changes: Vec<GerritChange> = self.get_json(&query.repository, &path)?;
        let has_more = changes.len() > page_size;
        let base = web_base(&query.repository);
        let pull_requests = changes
            .into_iter()
            .take(page_size)
            .map(|change| change.into_summary(&query.repository, &base))
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
        let change = self.fetch_change(&repository, target.number)?;
        let base = web_base(&repository);
        Ok(change.into_details(&repository, &base))
    }

    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<Vec<FilePatch>> {
        // A Gerrit change is a single commit, so its diff is exactly
        // `parent..revision` — no merge-base walk needed.
        self.local_diff(pr, &pr.base_sha, &pr.head_sha)
    }

    fn get_pull_request_commit_range_diff(
        &self,
        pr: &PullRequestDetails,
        start_sha: &str,
        end_sha: &str,
    ) -> Result<Vec<FilePatch>> {
        self.local_diff(pr, start_sha, end_sha)
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
        // The change-level endpoint returns every published comment across all
        // patch sets, so threads from earlier patch sets still render (marked
        // outdated) instead of disappearing on the next push.
        let comments: BTreeMap<String, Vec<GerritComment>> =
            self.get_json(&pr.repository, &format!("/changes/{}/comments", pr.number))?;
        Ok(threads_from_comment_map(
            comments,
            &pr.url,
            patch_set_from_ref(&pr.head_ref_name),
        ))
    }

    fn list_pull_request_commits(&self, pr: &PullRequestDetails) -> Result<Vec<PullRequestCommit>> {
        let change = self.fetch_change(&pr.repository, pr.number)?;
        Ok(change.into_commits())
    }

    fn create_review(
        &self,
        pr: &PullRequestDetails,
        request: CreateReviewRequest<'_>,
    ) -> Result<GhCreateReviewResponse> {
        if !self.http.is_authenticated() {
            return Err(TuicrError::Forge(format!(
                "Submitting to Gerrit needs credentials. Set {USER_ENV_VAR} and \
                 {PASSWORD_ENV_VAR} (Settings → HTTP Credentials in Gerrit)."
            )));
        }

        if request.event == SubmitEvent::Draft {
            self.create_drafts(pr, &request)?;
            return Ok(GhCreateReviewResponse {
                id: pr.number,
                html_url: pr.url.clone(),
                state: "PENDING".to_string(),
            });
        }

        // Gerrit takes the whole review — message, vote, and every inline
        // comment — in a single POST, keyed by file path.
        let mut by_path: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for comment in request.comments {
            by_path
                .entry(comment_path_key(comment))
                .or_default()
                .push(Value::Object(comment_input(comment)));
        }

        let mut payload = Map::new();
        if !request.body.is_empty() {
            payload.insert("message".to_string(), json!(request.body));
        }
        if !by_path.is_empty() {
            payload.insert("comments".to_string(), json!(by_path));
        }
        if let Some(vote) = review_vote(request.event) {
            payload.insert("labels".to_string(), json!({ REVIEW_LABEL: vote }));
        }
        // Publishing a review would otherwise sweep up unrelated drafts the
        // user left in the Gerrit web UI.
        payload.insert("drafts".to_string(), Value::String("KEEP".to_string()));

        self.send(
            &pr.repository,
            "POST",
            &format!("/changes/{}/revisions/{}/review", pr.number, pr.head_sha),
            &serde_json::to_string(&Value::Object(payload))?,
        )?;

        Ok(GhCreateReviewResponse {
            id: pr.number,
            html_url: pr.url.clone(),
            state: match request.event {
                SubmitEvent::Approve => "APPROVED",
                SubmitEvent::RequestChanges => "CHANGES_REQUESTED",
                SubmitEvent::Comment | SubmitEvent::Draft => "COMMENTED",
            }
            .to_string(),
        })
    }
}

/// Patch-set number carried by a change ref (`refs/changes/65/3965/2` → 2).
///
/// Cheaper than re-fetching the change just to learn which patch set is
/// current; `0` (no patch set) when the ref has an unexpected shape, which
/// leaves every thread marked current rather than falsely outdated.
fn patch_set_from_ref(head_ref: &str) -> u32 {
    head_ref
        .rsplit('/')
        .next()
        .and_then(|segment| segment.parse().ok())
        .unwrap_or(0)
}

/// The `Code-Review` vote a submit event casts, if any.
///
/// `+2` is Gerrit's "approved, may be submitted" and `-1` its "prefer this is
/// not merged as is" — `-2` is a hard veto, which is stronger than what
/// "request changes" means on the other forges.
fn review_vote(event: SubmitEvent) -> Option<i32> {
    match event {
        SubmitEvent::Approve => Some(2),
        SubmitEvent::RequestChanges => Some(-1),
        SubmitEvent::Comment | SubmitEvent::Draft => None,
    }
}

/// The path a comment files under. Gerrit keys base-side comments by the
/// pre-rename path.
fn comment_path(comment: &InlineComment) -> &Path {
    match (comment.side, comment.old_path.as_ref()) {
        (GhSide::Left, Some(old)) => old.as_path(),
        _ => comment.path.as_path(),
    }
}

/// That path as the JSON string Gerrit keys comments by — the map key when
/// reviewing, a field when drafting.
fn comment_path_key(comment: &InlineComment) -> String {
    comment_path(comment).to_string_lossy().into_owned()
}

/// Build Gerrit's `CommentInput` for one inline comment, minus the path.
fn comment_input(comment: &InlineComment) -> Map<String, Value> {
    let side = match comment.side {
        GhSide::Left => "PARENT",
        GhSide::Right => "REVISION",
    };
    let mut input = Map::new();
    input.insert("line".to_string(), json!(comment.line));
    input.insert("message".to_string(), json!(comment.body));
    input.insert("side".to_string(), json!(side));
    input.insert("unresolved".to_string(), json!(true));
    // Gerrit requires `line == range.end_line`, so only a real multi-line
    // selection gets a range.
    if let Some(start) = comment.start_line.filter(|start| *start < comment.line) {
        input.insert(
            "range".to_string(),
            json!({
                "start_line": start,
                "start_character": 0,
                "end_line": comment.line,
                "end_character": 0,
            }),
        );
    }
    input
}

// ---------- Local git helpers (mirror az) ----------

fn sha_present(root: &Path, sha: &str) -> bool {
    if sha.is_empty() {
        return false;
    }
    let spec = format!("{sha}^{{commit}}");
    run_command_output("git", Some(root), ["cat-file", "-e", spec.as_str()]).is_ok()
}

/// Read a git blob via `git show <sha>:<path>`. `None` on any failure so
/// callers fall back to the REST API.
fn read_blob(root: &Path, sha: &str, path: &Path) -> Option<String> {
    let spec = format!("{}:{}", sha, path.to_string_lossy());
    run_command_output("git", Some(root), ["show", spec.as_str()]).ok()
}

/// Name of the remote in `root` that points at `repo`, preferring `origin`.
fn remote_for_repository(root: &Path, repo: &ForgeRepository) -> Option<String> {
    let git_repo = git2::Repository::discover(root).ok()?;
    let matches = |name: &str| {
        git_repo
            .find_remote(name)
            .ok()
            .and_then(|remote| remote.url().map(str::to_string))
            .and_then(|url| parse_gerrit_remote_url(&url))
            .is_some_and(|parsed| &parsed == repo)
    };
    if matches("origin") {
        return Some("origin".to_string());
    }
    git_repo
        .remotes()
        .ok()?
        .iter()
        .flatten()
        .find(|name| matches(name))
        .map(str::to_string)
}

fn missing_checkout() -> TuicrError {
    TuicrError::Forge(
        "Reviewing a Gerrit change needs a local clone of the project (Gerrit's patch endpoint \
         cannot be paired with authoritative file metadata). Run tuicr from inside a clone."
            .to_string(),
    )
}

fn short(sha: &str) -> String {
    sha.chars().take(8).collect()
}

/// `server` is the base URL actually contacted, not `repo.host` — `GERRIT_URL`
/// can point them at different hosts, and naming the remote's host in a
/// failure from the configured server sends the reader to the wrong place.
fn map_http_error(error: GerritHttpError, server: &str) -> TuicrError {
    match error {
        GerritHttpError::Auth(detail) => {
            let hint = if credentials_from_env().is_some() {
                format!(
                    "Gerrit rejected the credentials in {USER_ENV_VAR}/{PASSWORD_ENV_VAR}. \
                     {PASSWORD_ENV_VAR} must hold the HTTP password from Settings → HTTP \
                     Credentials, not your account password."
                )
            } else {
                format!(
                    "Gerrit needs authentication for this request. Set {USER_ENV_VAR} and \
                     {PASSWORD_ENV_VAR} (Settings → HTTP Credentials in Gerrit)."
                )
            };
            TuicrError::Forge(format!("{hint}\n{}", trim_detail(&detail)))
        }
        GerritHttpError::Failed { status, body } => {
            let status = status.map(|s| format!(" (HTTP {s})")).unwrap_or_default();
            TuicrError::Forge(format!(
                "Gerrit request to {server} failed{status}: {}",
                trim_detail(&body)
            ))
        }
    }
}

/// Keep error detail readable: collapse whitespace and cap the length.
///
/// The cap counts characters, not bytes: Gerrit error bodies are often
/// non-ASCII HTML, and slicing at a byte offset panics when it lands inside a
/// multi-byte character.
fn trim_detail(detail: &str) -> String {
    let collapsed = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    match collapsed.char_indices().nth(MAX_DETAIL_CHARS) {
        Some((end, _)) => format!("{}…", &collapsed[..end]),
        None => collapsed,
    }
}

/// Percent-encode a file path for a Gerrit URL path segment. Gerrit expects
/// the whole path in one segment, so `/` is encoded too.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.replace('\\', "/").bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Encode a value used inside a `q=` query term. Gerrit reads `/` literally in
/// project names, so only the characters that would break the URL are escaped.
fn encode_query_term(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace(' ', "%20")
        .replace('+', "%2B")
        .replace('&', "%26")
        .replace('#', "%23")
        .replace('?', "%3F")
}

// ---------- URL & target parsing ----------

/// True when a remote points at a Gerrit instance.
///
/// Self-hosted Gerrit has no reserved domain, so detection leans on three
/// signals, in order of confidence: the canonical SSH port `29418`, a
/// `GERRIT_URL` naming this host, and a hostname containing "gerrit" (the
/// convention most deployments follow, and the same heuristic the GitLab
/// backend uses for self-hosted instances).
fn is_gerrit_remote(host: &str, port: Option<&str>) -> bool {
    if port == Some(GERRIT_SSH_PORT) {
        return true;
    }
    let host = host.to_ascii_lowercase();
    configured_host().is_some_and(|known| known == host) || host.contains("gerrit")
}

/// Parse a Gerrit remote (git) URL into a `ForgeRepository`.
///
/// Accepts `ssh://user@host:29418/project/path`, `https://host/project/path`,
/// and the `https://host/a/project` form Gerrit serves for authenticated
/// clones. Returns `None` for hosts that don't look like Gerrit.
pub fn parse_gerrit_remote_url(remote_url: &str) -> Option<ForgeRepository> {
    let trimmed = trim_url_suffix(remote_url.trim());
    if trimmed.is_empty() {
        return None;
    }

    // SCP-like SSH: `user@host:project`. There is no port to key on, so this
    // form relies on the hostname or `GERRIT_URL`.
    if let Some((host, path)) = parse_scp_like_remote(trimmed) {
        if !is_gerrit_remote(host, None) {
            return None;
        }
        return repository_from_path(host, path);
    }

    let (host_port, path) = strip_scheme_and_userinfo(trimmed).split_once('/')?;
    let (host, port) = split_host_port(host_port);
    if !is_gerrit_remote(host, port) {
        return None;
    }
    repository_from_path(host, strip_configured_path_prefix(path))
}

/// Drop the `GERRIT_URL` path prefix from a remote's path. When Gerrit is
/// served under a subdirectory that prefix belongs to the server URL, not to
/// the project name.
fn strip_configured_path_prefix(path: &str) -> &str {
    let Some(prefix) = configured_path_prefix() else {
        return path;
    };
    path.strip_prefix(prefix.as_str())
        .map_or(path, |rest| rest.trim_start_matches('/'))
}

fn repository_from_path(host: &str, path: &str) -> Option<ForgeRepository> {
    // Gerrit serves authenticated git over `/a/<project>`; the prefix is
    // transport, not part of the project name.
    let path = path.strip_prefix("a/").unwrap_or(path);
    let project = strip_git_suffix(path.trim_matches('/'));
    if project.is_empty() {
        return None;
    }
    Some(ForgeRepository::gerrit(host.to_ascii_lowercase(), project))
}

/// Parse a change target: a bare number or a Gerrit change URL.
///
/// Handles the modern `/c/<project>/+/<number>` shape and the legacy
/// `/#/c/<number>` one, with an optional trailing patch-set segment.
pub fn parse_pull_request_target_gerrit(input: &str) -> Result<PullRequestTarget> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return malformed_target(input);
    }
    if let Some(number) = positive_number(trimmed) {
        return Ok(PullRequestTarget::number(number, trimmed));
    }
    if let Some(target) = parse_gerrit_url_target(trimmed) {
        return Ok(target);
    }
    malformed_target(input)
}

fn parse_gerrit_url_target(target: &str) -> Option<PullRequestTarget> {
    // A change target must be a URL; a scheme-less value is not one.
    strip_scheme(target)?;
    let (host_port, path) = strip_scheme_and_userinfo(target).split_once('/')?;
    let (host, port) = split_host_port(host_port);
    if !is_gerrit_remote(host, port) {
        return None;
    }

    let segments: Vec<&str> = trim_url_suffix(path)
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "#")
        .collect();
    let change_at = segments.iter().position(|segment| *segment == "c")?;
    // Modern: c/<project…>/+/<n>. Legacy: c/<n> (project omitted).
    let (project, number) = match segments.iter().position(|segment| *segment == "+") {
        Some(plus) if plus > change_at => (
            segments[change_at + 1..plus].join("/"),
            positive_number(segments.get(plus + 1)?)?,
        ),
        _ => (
            String::new(),
            positive_number(segments.get(change_at + 1)?)?,
        ),
    };

    if project.is_empty() {
        Some(PullRequestTarget::number(number, target))
    } else {
        Some(PullRequestTarget::with_repository(
            ForgeRepository::gerrit(host.to_ascii_lowercase(), project),
            number,
            target,
        ))
    }
}

fn positive_number(value: &str) -> Option<u64> {
    if !value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok().filter(|number| *number > 0)
}

fn malformed_target<T>(input: &str) -> Result<T> {
    Err(TuicrError::Forge(format!(
        "Malformed Gerrit change target: `{input}`"
    )))
}

// ---------- Small URL helpers (mirror az) ----------

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

/// Reduce a URL to `host[:port]/path…` by dropping the scheme and any
/// `user@` / `user:password@` userinfo. Both are optional, so a bare
/// `host/path` passes through untouched.
fn strip_scheme_and_userinfo(value: &str) -> &str {
    let without_scheme = strip_scheme(value).unwrap_or(value);
    without_scheme
        .rsplit_once('@')
        .map_or(without_scheme, |(_, rest)| rest)
}

/// Split an authority into its host and its port, if it carries a numeric one.
/// `host:notaport` is left whole — a colon alone does not make a port.
fn split_host_port(authority: &str) -> (&str, Option<&str>) {
    match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            (host, Some(port))
        }
        _ => (authority, None),
    }
}

fn trim_url_suffix(value: &str) -> &str {
    let without_query = value.split_once('?').map_or(value, |(head, _)| head);
    without_query.trim_end_matches('/')
}

fn strip_git_suffix(value: &str) -> &str {
    value.strip_suffix(".git").unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// `GERRIT_*` env vars are process-global, so the tests that set them run
    /// one at a time.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn gerrit_repo() -> ForgeRepository {
        ForgeRepository::gerrit("gerrit.example.com", "platform/base")
    }

    // ---- Coordinate packing ----

    #[test]
    fn should_round_trip_multi_segment_projects_through_owner_packing() {
        // given/when
        let repo = ForgeRepository::gerrit("gerrit.example.com", "platform/frameworks/base");
        // then
        assert_eq!(repo.owner, "platform/frameworks");
        assert_eq!(repo.name, "base");
        assert_eq!(gerrit_project(&repo), "platform/frameworks/base");
    }

    #[test]
    fn should_fall_back_to_the_host_as_owner_for_single_segment_projects() {
        // given/when — an empty owner would make `ge:/myrepo/pr/1` unparseable
        let repo = ForgeRepository::gerrit("gerrit.example.com", "myrepo");
        // then
        assert_eq!(repo.owner, "gerrit.example.com");
        assert_eq!(gerrit_project(&repo), "myrepo");
        assert_eq!(repo.display_name(), "gerrit.example.com/myrepo");
    }

    // ---- URL parsing ----

    #[test]
    fn should_parse_ssh_remotes_on_the_canonical_gerrit_port() {
        // given/when — the host says nothing; port 29418 does
        let repo = parse_gerrit_remote_url("ssh://jdoe@review.internal:29418/platform/base");
        // then
        assert_eq!(
            repo,
            Some(ForgeRepository::gerrit("review.internal", "platform/base"))
        );
    }

    #[test]
    fn should_parse_https_remotes_on_a_gerrit_hostname() {
        for url in [
            "https://gerrit.example.com/platform/base",
            "https://gerrit.example.com/platform/base.git",
            "https://jdoe@gerrit.example.com/a/platform/base",
        ] {
            assert_eq!(
                parse_gerrit_remote_url(url),
                Some(gerrit_repo()),
                "{url} should parse"
            );
        }
    }

    #[test]
    fn should_reject_remotes_that_do_not_look_like_gerrit() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::remove_var(URL_ENV_VAR) };
        assert_eq!(
            parse_gerrit_remote_url("https://github.com/agavra/tuicr"),
            None
        );
        assert_eq!(
            parse_gerrit_remote_url("git@bitbucket.org:workspace/repo.git"),
            None
        );
    }

    #[test]
    fn should_recognize_a_configured_host_and_strip_its_path_prefix() {
        // given — Gerrit served under a subdirectory on a neutral hostname
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::set_var(URL_ENV_VAR, "https://review.internal/gerrit/") };
        // when
        let repo = parse_gerrit_remote_url("https://review.internal/gerrit/platform/base");
        // then — the prefix belongs to the server URL, not the project
        assert_eq!(
            repo,
            Some(ForgeRepository::gerrit("review.internal", "platform/base"))
        );
        assert_eq!(
            web_base(&repo.expect("parsed")),
            "https://review.internal/gerrit"
        );
        unsafe { std::env::remove_var(URL_ENV_VAR) };
    }

    #[test]
    fn should_default_a_schemeless_gerrit_url_to_https() {
        // given — a bare host used to reach the HTTP client verbatim, which
        // rejected it with `http: invalid format`
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::set_var(URL_ENV_VAR, "review.internal/gerrit/") };
        // when/then
        assert_eq!(
            configured_base_url().as_deref(),
            Some("https://review.internal/gerrit")
        );
        unsafe { std::env::remove_var(URL_ENV_VAR) };
    }

    #[test]
    fn should_normalize_an_ssh_gerrit_url_onto_https_without_its_port() {
        // given — an SSH URL pasted from `git remote -v`; 29418 is the SSH
        // port and must not be carried onto HTTPS
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::set_var(URL_ENV_VAR, "ssh://jdoe@review.internal:29418") };
        // when/then
        assert_eq!(
            configured_base_url().as_deref(),
            Some("https://review.internal")
        );
        unsafe { std::env::remove_var(URL_ENV_VAR) };
    }

    #[test]
    fn should_keep_a_deliberate_port_on_a_schemeless_gerrit_url() {
        // given — only Gerrit's SSH port is meaningless over HTTPS; dropping
        // every port silently turned `127.0.0.1:8731` into `https://127.0.0.1`
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::set_var(URL_ENV_VAR, "review.internal:8443/gerrit") };
        // when/then
        assert_eq!(
            configured_base_url().as_deref(),
            Some("https://review.internal:8443/gerrit")
        );
        unsafe { std::env::remove_var(URL_ENV_VAR) };
    }

    #[test]
    fn should_keep_an_explicit_http_url_with_its_port_and_prefix() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::set_var(URL_ENV_VAR, "http://review.internal:8080/gerrit") };
        assert_eq!(
            configured_base_url().as_deref(),
            Some("http://review.internal:8080/gerrit")
        );
        unsafe { std::env::remove_var(URL_ENV_VAR) };
    }

    #[test]
    fn should_use_gerrit_url_when_the_web_host_differs_from_the_git_remote_host() {
        // given — the SSH gateway in the remote is not the web host, the one
        // deployment shape a remote URL cannot describe
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::set_var(URL_ENV_VAR, "https://review.corp.com") };
        let repo = parse_gerrit_remote_url("ssh://jdoe@gerrit-ssh.corp.com:29418/platform/base")
            .expect("parsed");
        // when/then — identity still follows the remote, but requests go to
        // the configured server
        assert_eq!(repo.host, "gerrit-ssh.corp.com");
        assert_eq!(web_base(&repo), "https://review.corp.com");
        unsafe { std::env::remove_var(URL_ENV_VAR) };
    }

    // ---- Target parsing ----

    #[test]
    fn should_parse_a_bare_change_number() {
        // given/when
        let target = parse_pull_request_target_gerrit("3965").expect("parse");
        // then
        assert_eq!(target.number, 3965);
        assert_eq!(target.repository, None);
    }

    #[test]
    fn should_parse_a_modern_change_url_with_its_project() {
        // given/when
        let target =
            parse_pull_request_target_gerrit("https://gerrit.example.com/c/platform/base/+/3965")
                .expect("parse");
        // then
        assert_eq!(target.number, 3965);
        assert_eq!(target.repository, Some(gerrit_repo()));
    }

    #[test]
    fn should_parse_a_legacy_change_url_without_a_project() {
        // given/when
        let target = parse_pull_request_target_gerrit("https://gerrit.example.com/#/c/3965/")
            .expect("parse");
        // then
        assert_eq!(target.number, 3965);
        assert_eq!(target.repository, None);
    }

    #[test]
    fn should_reject_targets_that_are_not_gerrit_changes() {
        for target in ["", "not-a-change", "https://github.com/agavra/tuicr/pull/5"] {
            assert!(
                parse_pull_request_target_gerrit(target).is_err(),
                "{target} should not parse"
            );
        }
    }

    // ---- Transport plumbing ----

    #[test]
    fn should_strip_the_xssi_prefix_before_parsing_json() {
        assert_eq!(strip_magic_prefix(")]}'\n[]"), "[]");
        assert_eq!(strip_magic_prefix("[]"), "[]");
    }

    #[test]
    fn should_cap_error_detail_without_splitting_a_multi_byte_character() {
        // given — a body whose 400th *byte* falls inside a 3-byte character.
        // Capping by byte offset panicked here, so a long non-ASCII error
        // page from Gerrit took the whole TUI down instead of reporting.
        let detail = format!("{}{}", "a".repeat(399), "€".repeat(10));
        // when
        let trimmed = trim_detail(&detail);
        // then — 400 characters kept, plus the ellipsis
        assert_eq!(trimmed.chars().count(), MAX_DETAIL_CHARS + 1);
        assert!(trimmed.ends_with("a€…"), "unexpected tail: {trimmed}");
    }

    #[test]
    fn should_leave_short_detail_untrimmed_and_collapse_its_whitespace() {
        assert_eq!(trim_detail("  not\n  found\t "), "not found");
    }

    /// Records every request and replays canned responses in order.
    struct FakeHttp {
        authenticated: bool,
        responses: Mutex<Vec<String>>,
        calls: Mutex<Vec<(String, String, Option<String>)>>,
    }

    impl FakeHttp {
        fn new(authenticated: bool, responses: Vec<&str>) -> Self {
            Self {
                authenticated,
                responses: Mutex::new(responses.into_iter().rev().map(str::to_string).collect()),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl FakeHttp {
        fn calls(&self) -> Vec<(String, String, Option<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    // Implemented on the `Arc` so a test can keep a handle on the recorder
    // after handing the transport to the backend.
    impl GerritHttp for Arc<FakeHttp> {
        fn is_authenticated(&self) -> bool {
            self.authenticated
        }

        fn request(&self, method: &str, url: &str, body: Option<&str>) -> GerritHttpResult<String> {
            self.calls.lock().unwrap().push((
                method.to_string(),
                url.to_string(),
                body.map(str::to_string),
            ));
            Ok(self.responses.lock().unwrap().pop().unwrap_or_default())
        }
    }

    /// A backend wired to a fresh recording transport, plus the recorder.
    fn backend_with(authenticated: bool, responses: Vec<&str>) -> (GerritBackend, Arc<FakeHttp>) {
        let fake = Arc::new(FakeHttp::new(authenticated, responses));
        let backend =
            GerritBackend::with_transport(Some(gerrit_repo()), Box::new(Arc::clone(&fake)));
        (backend, fake)
    }

    fn inline_comment(line: u32, start_line: Option<u32>, side: GhSide) -> InlineComment {
        InlineComment {
            path: PathBuf::from("src/main.rs"),
            line,
            side,
            counterpart_line: None,
            start_line,
            start_side: None,
            range_anchors: None,
            old_path: None,
            body: "looks wrong".to_string(),
            comment_id: "local-1".to_string(),
        }
    }

    fn details() -> PullRequestDetails {
        PullRequestDetails {
            repository: gerrit_repo(),
            number: 3965,
            title: "Implement feature X".to_string(),
            url: "https://gerrit.example.com/c/platform/base/+/3965".to_string(),
            state: "OPEN".to_string(),
            is_draft: false,
            author: None,
            head_ref_name: "refs/changes/65/3965/2".to_string(),
            base_ref_name: "main".to_string(),
            head_sha: "abc1234".to_string(),
            base_sha: "def5678".to_string(),
            body: String::new(),
            updated_at: None,
            closed: false,
            merged_at: None,
            diff_start_sha: None,
        }
    }

    #[test]
    fn should_prefix_authenticated_requests_with_the_gerrit_a_path() {
        // given
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::remove_var(URL_ENV_VAR) };
        let (backend, http) = backend_with(true, vec![")]}'\n[]"]);
        // when
        let _ = backend.list_pull_requests(PullRequestListQuery::first_page(gerrit_repo(), 10));
        // then
        let url = &http.calls()[0].1;
        assert!(
            url.starts_with("https://gerrit.example.com/a/changes/?q="),
            "unexpected url: {url}"
        );
        assert!(
            url.contains("project:platform/base"),
            "unexpected url: {url}"
        );
    }

    #[test]
    fn should_scope_the_requested_list_to_the_signed_in_users_attention_set() {
        // given
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::remove_var(URL_ENV_VAR) };
        let (backend, http) = backend_with(true, vec![")]}'\n[]"]);
        // when
        let _ = backend.list_pull_requests(PullRequestListQuery::first_page_with_scope(
            gerrit_repo(),
            10,
            PullRequestListScope::ReviewRequested,
        ));
        // then — the attention set, minus changes the user owns
        let url = &http.calls()[0].1;
        assert!(url.contains("attention:self"), "unexpected url: {url}");
        assert!(url.contains("-owner:self"), "unexpected url: {url}");
        assert!(
            !url.contains("reviewer:self"),
            "reviewer:self would also match changes already handed back to the author"
        );
    }

    #[test]
    fn should_omit_the_attention_filter_when_anonymous() {
        // given — `attention:self` is meaningless without a session
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::remove_var(URL_ENV_VAR) };
        let (backend, http) = backend_with(false, vec![")]}'\n[]"]);
        // when
        let _ = backend.list_pull_requests(PullRequestListQuery::first_page_with_scope(
            gerrit_repo(),
            10,
            PullRequestListScope::ReviewRequested,
        ));
        // then
        let url = &http.calls()[0].1;
        assert!(!url.contains("attention:self"), "unexpected url: {url}");
        assert!(
            url.starts_with("https://gerrit.example.com/changes/?q="),
            "anonymous calls skip the /a prefix, got: {url}"
        );
    }

    #[test]
    fn should_post_one_review_carrying_the_message_vote_and_comments() {
        // given
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::remove_var(URL_ENV_VAR) };
        let (backend, http) = backend_with(true, vec![")]}'\n{}"]);
        let comments = vec![
            inline_comment(12, None, GhSide::Right),
            inline_comment(20, Some(18), GhSide::Left),
        ];
        // when
        let response = backend
            .create_review(
                &details(),
                CreateReviewRequest {
                    event: SubmitEvent::Approve,
                    commit_id: "abc1234",
                    body: "LGTM",
                    comments: &comments,
                },
            )
            .expect("review");
        // then
        let calls = http.calls();
        assert_eq!(calls.len(), 1, "the whole review is one POST");
        assert_eq!(calls[0].0, "POST");
        assert_eq!(
            calls[0].1,
            "https://gerrit.example.com/a/changes/3965/revisions/abc1234/review"
        );
        let payload: Value = serde_json::from_str(calls[0].2.as_deref().expect("body")).unwrap();
        assert_eq!(payload["message"], "LGTM");
        assert_eq!(payload["labels"]["Code-Review"], 2);
        assert_eq!(payload["drafts"], "KEEP");
        let file = &payload["comments"]["src/main.rs"];
        assert_eq!(file[0]["line"], 12);
        assert_eq!(file[0]["side"], "REVISION");
        assert!(
            file[0]["range"].is_null(),
            "single-line comments carry no range"
        );
        assert_eq!(file[1]["side"], "PARENT");
        assert_eq!(file[1]["range"]["start_line"], 18);
        assert_eq!(file[1]["range"]["end_line"], 20);
        assert_eq!(response.state, "APPROVED");
        assert_eq!(response.html_url, details().url);
    }

    #[test]
    fn should_vote_minus_one_rather_than_veto_when_requesting_changes() {
        // given — -2 is a hard block, stronger than "request changes" elsewhere
        assert_eq!(review_vote(SubmitEvent::RequestChanges), Some(-1));
        assert_eq!(review_vote(SubmitEvent::Approve), Some(2));
        assert_eq!(review_vote(SubmitEvent::Comment), None);
    }

    #[test]
    fn should_send_draft_reviews_to_the_gerrit_drafts_endpoint() {
        // given
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::remove_var(URL_ENV_VAR) };
        let (backend, http) = backend_with(true, vec![")]}'\n{}", ")]}'\n{}"]);
        let comments = vec![inline_comment(12, None, GhSide::Right)];
        // when
        let response = backend
            .create_review(
                &details(),
                CreateReviewRequest {
                    event: SubmitEvent::Draft,
                    commit_id: "abc1234",
                    body: "still thinking",
                    comments: &comments,
                },
            )
            .expect("draft");
        // then — one draft for the review body, one per inline comment
        let calls = http.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|(method, url, _)| {
            method == "PUT"
                && url == "https://gerrit.example.com/a/changes/3965/revisions/abc1234/drafts"
        }));
        let body: Value = serde_json::from_str(calls[0].2.as_deref().expect("body")).unwrap();
        assert_eq!(body["path"], "/PATCHSET_LEVEL");
        assert_eq!(body["message"], "still thinking");
        let inline: Value = serde_json::from_str(calls[1].2.as_deref().expect("body")).unwrap();
        assert_eq!(inline["path"], "src/main.rs");
        assert_eq!(inline["line"], 12);
        assert_eq!(response.state, "PENDING");
    }

    #[test]
    fn should_read_review_threads_from_the_change_level_comments_endpoint() {
        // given — a comment left on patch set 1 while the change is on 2
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::remove_var(URL_ENV_VAR) };
        let (backend, http) = backend_with(
            true,
            vec![
                r#")]}'
                {"src/main.rs": [{"id": "c1", "line": 7, "message": "nit", "patch_set": 1,
                                  "updated": "2013-02-21 11:00:00.000000000"}]}"#,
            ],
        );
        // when
        let threads = backend.list_review_threads(&details()).expect("threads");
        // then — one request, and the patch set comes from the change ref
        let calls = http.calls();
        assert_eq!(calls.len(), 1, "the patch set is read from head_ref_name");
        assert_eq!(
            calls[0].1,
            "https://gerrit.example.com/a/changes/3965/comments"
        );
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].path, "src/main.rs");
        assert!(
            threads[0].is_outdated,
            "patch set 1 is behind the change's current 2"
        );
    }

    #[test]
    fn should_read_patch_set_numbers_off_the_change_ref() {
        assert_eq!(patch_set_from_ref("refs/changes/65/3965/2"), 2);
        assert_eq!(patch_set_from_ref("refs/changes/65/3965"), 3965);
        assert_eq!(patch_set_from_ref("main"), 0);
    }

    #[test]
    fn should_fetch_file_content_by_project_and_commit() {
        // given — a request with no local checkout to read the blob from
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        unsafe { std::env::remove_var(URL_ENV_VAR) };
        let (backend, http) = backend_with(true, vec![&BASE64.encode("one\ntwo\nthree\n")]);
        let request = ForgeFileLinesRequest {
            repository: gerrit_repo(),
            base_sha: "def5678".to_string(),
            head_sha: "abc1234".to_string(),
            path: PathBuf::from("src/deep/main.rs"),
            status: crate::model::FileStatus::Modified,
            side: crate::forge::traits::ForgeFileSide::Head,
            start_line: 1,
            end_line: 2,
        };
        // when
        let lines = backend.fetch_file_lines(request).expect("lines");
        // then — the project/commit shape, with both paths fully encoded
        assert_eq!(
            http.calls()[0].1,
            "https://gerrit.example.com/a/projects/platform%2Fbase/commits/abc1234\
             /files/src%2Fdeep%2Fmain.rs/content"
        );
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn backend_is_send_and_sync() {
        // `fetch_pr_data` takes `&dyn ForgeBackend` on a background thread.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GerritBackend>();
    }

    #[test]
    fn should_refuse_to_submit_without_credentials() {
        // given
        let (backend, _http) = backend_with(false, vec![]);
        // when
        let error = backend
            .create_review(
                &details(),
                CreateReviewRequest {
                    event: SubmitEvent::Comment,
                    commit_id: "abc1234",
                    body: "hi",
                    comments: &[],
                },
            )
            .expect_err("should refuse");
        // then
        assert!(
            error.to_string().contains(USER_ENV_VAR),
            "unexpected error: {error}"
        );
    }
}
