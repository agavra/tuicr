//! Bitbucket Cloud backend driven by the `bkt` CLI.
//!
//! Most reads go through `bkt api`, which passes the REST 2.0 response
//! straight through. That gets us full 40-char commit hashes, `resolution` /
//! `outdated` flags on comments, and explicit pagination control — none of
//! which `bkt`'s formatted output exposes. The patch itself comes from
//! `bkt pr diff`, whose output already carries `diff --git` headers.
//!
//! Two Bitbucket-isms are handled here rather than leaking upward:
//! - **Abbreviated hashes.** PR payloads report 12-char commit hashes while
//!   the commits endpoint reports 40. `promote_sha` widens them so a
//!   `PrSessionKey` stays stable and commit-scope comparisons line up.
//! - **Reversed diff specs.** Bitbucket's `/diff/{spec}` takes `new..old`,
//!   the opposite of `git diff old..new`. Verified against the live API.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::forge::remote_comments::{RemoteReviewSummary, RemoteReviewThread};
use crate::forge::submit::{GhSide, SubmitEvent};
use crate::forge::traits::{
    CreateReviewRequest, ForgeBackend, ForgeFileLinesRequest, ForgeRepository,
    GhCreateReviewResponse, PagedPullRequests, PullRequestCommit, PullRequestDetails,
    PullRequestListQuery, PullRequestListScope, PullRequestReviewMetadata, PullRequestTarget,
};
use crate::model::{DiffLine, FilePatch};
use crate::process::{CommandOutputError, CommandOutputErrorKind, run_command_output};
use crate::vcs::git::raw::{pair_metadata_with_patch, run_git_diff};
use crate::vcs::slice_context_lines;

use super::models::{
    BbComment, BbCommit, BbDiffStat, BbPaged, BbPullRequest, BbUser, group_into_review_threads,
    review_summaries,
};

/// The only Bitbucket host this backend claims. Data Center instances live on
/// arbitrary hostnames and speak REST 1.0, so they are deliberately excluded.
const BITBUCKET_CLOUD_HOST: &str = "bitbucket.org";

/// Cloud's maximum page size for most collections.
const MAX_PAGE_LEN: usize = 100;

/// The `pullrequests` collection caps `pagelen` lower than the rest of the API
/// and rejects anything larger with `400 Invalid pagelen`.
const MAX_PR_LIST_PAGE_LEN: usize = 50;

/// Ceiling on pages walked when draining a collection, so a pathological
/// repository can't spin forever.
const MAX_PAGES: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BktCommandError {
    MissingBkt,
    Failed { status: Option<i32>, stderr: String },
}

pub type BktCommandResult<T> = std::result::Result<T, BktCommandError>;

pub trait BktCommandRunner {
    fn run(&self, args: &[String]) -> BktCommandResult<String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemBktRunner;

impl BktCommandRunner for SystemBktRunner {
    fn run(&self, args: &[String]) -> BktCommandResult<String> {
        run_command_output("bkt", None, args.iter().map(|arg| OsStr::new(arg.as_str())))
            .map_err(BktCommandError::from)
    }
}

impl From<CommandOutputError> for BktCommandError {
    fn from(error: CommandOutputError) -> Self {
        match error.kind {
            CommandOutputErrorKind::NotFound => Self::MissingBkt,
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

/// Return `Some(diff)` when both SHAs exist locally, via `git diff <start>..<end>`.
fn local_range_diff(repo_root: &Path, start_sha: &str, end_sha: &str) -> Option<Vec<FilePatch>> {
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
    run_git_diff(repo_root, &[range.as_str()]).ok()
}

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

impl<R> BitbucketBktBackend<R>
where
    R: BktCommandRunner,
{
    pub fn with_runner(default_repository: Option<ForgeRepository>, runner: R) -> Self {
        Self {
            default_repository,
            runner,
            local_checkout: None,
        }
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
        self.runner.run(&args).map_err(map_bkt_error)
    }

    /// Workspace/repo selection flags.
    ///
    /// These are always passed explicitly rather than relying on `bkt`'s
    /// active context: the user's context may point at an unrelated
    /// workspace, and commands like `bkt pr comments` hard-fail without them.
    fn repo_args(repo: &ForgeRepository) -> Vec<String> {
        vec![
            "--workspace".to_string(),
            repo.owner.clone(),
            "--repo".to_string(),
            repo.name.clone(),
        ]
    }

    /// Base path for a repository's REST 2.0 resources.
    fn repo_path(repo: &ForgeRepository) -> String {
        format!("/2.0/repositories/{}/{}", repo.owner, repo.name)
    }

    /// Issue a `bkt api` GET. `params` become repeated `-P key=value` flags.
    fn run_api(&self, path: String, params: &[(&str, String)]) -> Result<String> {
        let mut args = vec!["api".to_string(), path];
        for (key, value) in params {
            args.push("-P".to_string());
            args.push(format!("{key}={value}"));
        }
        self.run_bkt(args)
    }

    /// The authenticated user's UUID.
    ///
    /// Cloud returns an empty `username` for everyone, so the UUID is the only
    /// usable identity. Not cached: it is needed on two cold paths only, and
    /// caching would make the backend non-`Sync` for the background threads
    /// that own it.
    fn viewer_uuid(&self) -> Result<Option<String>> {
        let output = self.run_api("/2.0/user".to_string(), &[])?;
        let user: BbUser = serde_json::from_str(&output)?;
        Ok(if user.uuid.is_empty() {
            None
        } else {
            Some(user.uuid)
        })
    }

    /// Widen an abbreviated commit hash to its full 40-char form.
    ///
    /// Best-effort: on any failure the input is returned unchanged, so a
    /// transient API error degrades the session rather than breaking it.
    fn promote_sha(&self, repo: &ForgeRepository, sha: &str) -> String {
        // Already full length (or empty, for a PR with no destination commit).
        if sha.len() >= 40 || sha.is_empty() {
            return sha.to_string();
        }
        let path = format!("{}/commit/{}", Self::repo_path(repo), sha);
        let Ok(output) = self.run_api(path, &[]) else {
            return sha.to_string();
        };
        match serde_json::from_str::<BbCommit>(&output) {
            Ok(commit) if commit.hash.starts_with(sha) => commit.hash,
            _ => sha.to_string(),
        }
    }

    /// Drain every page of a paginated collection, newest-first as Bitbucket
    /// returns them.
    ///
    /// Pages are walked by following the server's `next` URL rather than by
    /// incrementing a `page` parameter. Cloud is inconsistent about numbered
    /// pages — the pull request `commits` endpoint rejects an explicit `page`
    /// with `400 Invalid page` even for page 1, while `comments` accepts it —
    /// but every collection returns a usable `next` link, and `bkt api` takes
    /// a full URL as its path.
    fn collect_pages<T: serde::de::DeserializeOwned>(&self, path: String) -> Result<Vec<T>> {
        let mut all: Vec<T> = Vec::new();
        let mut next = Some(path);
        let mut fetched = 0;
        while let Some(target) = next.take() {
            // Only the first request needs an explicit page size; `next`
            // carries the original `pagelen` forward.
            let params: &[(&str, String)] = if fetched == 0 {
                &[("pagelen", MAX_PAGE_LEN.to_string())]
            } else {
                &[]
            };
            let output = self.run_api(target, params)?;
            let parsed: BbPaged<T> = serde_json::from_str(&output)?;
            all.extend(parsed.values);
            fetched += 1;
            if fetched >= MAX_PAGES {
                break;
            }
            next = parsed.next;
        }
        Ok(all)
    }

    fn list_comments(&self, pr: &PullRequestDetails) -> Result<Vec<BbComment>> {
        let path = format!(
            "{}/pullrequests/{}/comments",
            Self::repo_path(&pr.repository),
            pr.number
        );
        self.collect_pages(path)
    }

    fn fetch_file_via_api(&self, request: &ForgeFileLinesRequest) -> Result<String> {
        let path_str = request.path.to_string_lossy().replace('\\', "/");
        // `bkt api` percent-encodes nothing for us, but the `src` endpoint
        // takes the path as literal path segments, so it needs no encoding.
        let path = format!(
            "{}/src/{}/{}",
            Self::repo_path(&request.repository),
            request.sha(),
            path_str,
        );
        self.run_api(path, &[])
    }

    /// POST a comment and return its Bitbucket id.
    ///
    /// The JSON body travels as an argv value because `bkt api` has no stdin
    /// mode — `-d -` is parsed as literal JSON, not a stdin sentinel
    /// (verified against `bkt` 0.30.0).
    fn post_comment(
        &self,
        pr: &PullRequestDetails,
        body: &serde_json::Value,
    ) -> Result<Option<u64>> {
        let path = format!(
            "{}/pullrequests/{}/comments",
            Self::repo_path(&pr.repository),
            pr.number
        );
        let args = vec![
            "api".to_string(),
            path,
            "--method".to_string(),
            "POST".to_string(),
            "--input".to_string(),
            serde_json::to_string(body)?,
        ];
        let output = self.runner.run(&args).map_err(map_create_comment_error)?;
        // A missing/unparseable id is not fatal: the comment landed, we just
        // can't record its remote id against the local draft.
        Ok(serde_json::from_str::<BbComment>(&output)
            .ok()
            .map(|comment| comment.id))
    }

    /// Build the `inline` anchor for a review comment.
    ///
    /// `to` is the head-side line, `from` the base-side line. Multi-line
    /// selections add `start_to` / `start_from`; Bitbucket ignores a start
    /// that equals the end, so single-line comments simply omit it.
    fn inline_anchor(comment: &crate::forge::submit::InlineComment) -> serde_json::Value {
        let path = comment.path.to_string_lossy().replace('\\', "/");
        let mut inline = serde_json::json!({ "path": path });
        let (line_key, start_key) = match comment.side {
            GhSide::Right => ("to", "start_to"),
            GhSide::Left => ("from", "start_from"),
        };
        inline[line_key] = serde_json::Value::from(comment.line);
        if let Some(start_line) = comment.start_line
            && start_line != comment.line
        {
            inline[start_key] = serde_json::Value::from(start_line);
        }
        inline
    }
}

impl<R> ForgeBackend for BitbucketBktBackend<R>
where
    R: BktCommandRunner,
{
    fn list_pull_requests(&self, query: PullRequestListQuery) -> Result<PagedPullRequests> {
        let page_size = query.page_size.clamp(1, MAX_PR_LIST_PAGE_LEN);
        // Bitbucket pages are 1-based and fixed-width, so ask for the page
        // that contains the next unseen row rather than over-fetching.
        let page = query.already_loaded / page_size + 1;
        let mut params = vec![
            ("pagelen", page_size.to_string()),
            ("page", page.to_string()),
        ];
        match query.scope {
            PullRequestListScope::Open => params.push(("state", "OPEN".to_string())),
            PullRequestListScope::ReviewRequested => {
                // Cloud has no workspace-wide "review requested" endpoint; the
                // repository-scoped `q` filter is the equivalent.
                let uuid = self.viewer_uuid()?.ok_or_else(|| {
                    TuicrError::Forge(
                        "Could not determine the authenticated Bitbucket user.\n\
                         Run `bkt auth status`."
                            .to_string(),
                    )
                })?;
                // The state clause has to live *inside* `q`: Cloud silently
                // ignores a standalone `state` parameter once `q` is present,
                // which otherwise leaks merged and declined pull requests into
                // the list.
                params.push(("q", format!("state=\"OPEN\" AND reviewers.uuid=\"{uuid}\"")));
            }
        }
        let path = format!("{}/pullrequests", Self::repo_path(&query.repository));
        let output = self.run_api(path, &params)?;
        let parsed: BbPaged<BbPullRequest> = serde_json::from_str(&output)?;
        let has_more = parsed.has_more();
        let pull_requests = parsed
            .values
            .into_iter()
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
        let path = format!(
            "{}/pullrequests/{}",
            Self::repo_path(&repository),
            target.number
        );
        let output = self.run_api(path, &[])?;
        let pr: BbPullRequest = serde_json::from_str(&output)?;
        let mut details = pr.into_details(&repository)?;
        // Cloud reports 12-char hashes here but 40-char ones on the commits
        // endpoint; widen now so the session key and commit scoping agree.
        details.head_sha = self.promote_sha(&repository, &details.head_sha);
        details.base_sha = self.promote_sha(&repository, &details.base_sha);
        Ok(details)
    }

    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<Vec<FilePatch>> {
        let diffstat_path = format!(
            "{}/pullrequests/{}/diffstat",
            Self::repo_path(&pr.repository),
            pr.number
        );
        let metadata = self
            .collect_pages::<BbDiffStat>(diffstat_path)?
            .into_iter()
            .map(BbDiffStat::into_metadata)
            .collect::<Result<Vec<_>>>()?;
        let mut args = vec!["pr".to_string(), "diff".to_string(), pr.number.to_string()];
        args.extend(Self::repo_args(&pr.repository));
        let patch = self.run_bkt(args)?;
        pair_metadata_with_patch(metadata, patch.as_bytes())
    }

    fn local_checkout_path(&self) -> Option<PathBuf> {
        self.local_checkout.clone()
    }

    fn list_pull_request_commits(&self, pr: &PullRequestDetails) -> Result<Vec<PullRequestCommit>> {
        let path = format!(
            "{}/pullrequests/{}/commits",
            Self::repo_path(&pr.repository),
            pr.number
        );
        let rows: Vec<BbCommit> = self.collect_pages(path)?;
        // Bitbucket returns newest-first; the trait contract is oldest-first.
        Ok(rows
            .into_iter()
            .map(BbCommit::into_pull_request_commit)
            .rev()
            .collect())
    }

    fn list_pull_request_review_metadata(
        &self,
        pr: &PullRequestDetails,
    ) -> Result<PullRequestReviewMetadata> {
        // Approvals live on the PR payload's participants, so this is one
        // request rather than a dedicated reviews endpoint.
        let path = format!(
            "{}/pullrequests/{}",
            Self::repo_path(&pr.repository),
            pr.number
        );
        let output = self.run_api(path, &[])?;
        let payload: BbPullRequest = serde_json::from_str(&output)?;
        Ok(PullRequestReviewMetadata {
            // Identity is UUID-based to match the participant records.
            viewer_login: self.viewer_uuid().unwrap_or_default(),
            reviews: payload.review_records(),
        })
    }

    fn get_pull_request_commit_range_diff(
        &self,
        pr: &PullRequestDetails,
        start_sha: &str,
        end_sha: &str,
    ) -> Result<Vec<FilePatch>> {
        if let Some(root) = self.local_checkout.as_deref()
            && let Some(diff) = local_range_diff(root, start_sha, end_sha)
        {
            return Ok(diff);
        }
        // Bitbucket's diff spec is `new..old` — the reverse of git's
        // `old..new`. Verified against the live API: the git-style order
        // returns an empty diff.
        let diffstat_path = format!(
            "{}/diffstat/{}..{}",
            Self::repo_path(&pr.repository),
            end_sha,
            start_sha
        );
        let metadata = self
            .collect_pages::<BbDiffStat>(diffstat_path)?
            .into_iter()
            .map(BbDiffStat::into_metadata)
            .collect::<Result<Vec<_>>>()?;
        let path = format!(
            "{}/diff/{}..{}",
            Self::repo_path(&pr.repository),
            end_sha,
            start_sha
        );
        let patch = self.run_api(path, &[])?;
        pair_metadata_with_patch(metadata, patch.as_bytes())
    }

    fn list_review_threads(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewThread>> {
        Ok(group_into_review_threads(self.list_comments(pr)?))
    }

    fn list_review_summaries(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewSummary>> {
        // Bitbucket has no review object; general (non-inline) comments are
        // the closest analogue to a GitHub review body.
        Ok(review_summaries(&self.list_comments(pr)?))
    }

    fn fetch_file_lines(&self, request: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
        if request.start_line == 0 || request.start_line > request.end_line {
            return Ok(Vec::new());
        }
        let content = match self
            .local_checkout
            .as_deref()
            .and_then(|root| read_blob_with_repo(root, request.sha(), request.path.as_path()))
        {
            Some(content) => content,
            None => self.fetch_file_via_api(&request)?,
        };
        Ok(slice_context_lines(
            &content,
            request.start_line,
            request.end_line,
        ))
    }

    fn file_line_count(&self, request: ForgeFileLinesRequest) -> Result<u32> {
        let content = match self
            .local_checkout
            .as_deref()
            .and_then(|root| read_blob_with_repo(root, request.sha(), request.path.as_path()))
        {
            Some(content) => content,
            None => self.fetch_file_via_api(&request)?,
        };
        Ok(content.lines().count() as u32)
    }

    fn create_review(
        &self,
        pr: &PullRequestDetails,
        request: CreateReviewRequest<'_>,
    ) -> Result<GhCreateReviewResponse> {
        // Bitbucket has no pending-review primitive reachable through `bkt`,
        // and Cloud's request-changes endpoint is not yet wired up here.
        // Reject up front rather than silently downgrading to a plain comment.
        match request.event {
            SubmitEvent::Comment | SubmitEvent::Approve => {}
            SubmitEvent::RequestChanges => {
                return Err(TuicrError::UnsupportedOperation(
                    "Requesting changes is not supported for Bitbucket yet. \
                     Use `:submit` to post comments, then request changes in Bitbucket."
                        .to_string(),
                ));
            }
            SubmitEvent::Draft => {
                return Err(TuicrError::UnsupportedOperation(
                    "Draft (pending) reviews are not supported for Bitbucket yet. \
                     Use `:submit` to publish comments directly."
                        .to_string(),
                ));
            }
        }

        let mut first_comment_id: Option<u64> = None;

        // The review body becomes a general (non-inline) comment.
        if !request.body.is_empty() {
            let body = serde_json::json!({ "content": { "raw": request.body } });
            let id = self.post_comment(pr, &body)?;
            first_comment_id = first_comment_id.or(id);
        }

        for comment in request.comments {
            let body = serde_json::json!({
                "content": { "raw": comment.body },
                "inline": Self::inline_anchor(comment),
            });
            let id = self.post_comment(pr, &body)?;
            first_comment_id = first_comment_id.or(id);
        }

        let approved = request.event == SubmitEvent::Approve;
        if approved {
            let mut args = vec![
                "pr".to_string(),
                "approve".to_string(),
                pr.number.to_string(),
            ];
            args.extend(Self::repo_args(&pr.repository));
            self.runner.run(&args).map_err(map_approve_error)?;
        }

        // Synthesized: Bitbucket returns no review object to report.
        Ok(GhCreateReviewResponse {
            id: first_comment_id.unwrap_or(0),
            html_url: pr.url.clone(),
            state: if approved { "APPROVED" } else { "COMMENTED" }.to_string(),
        })
    }
}

/// Parse a pull request target string in Bitbucket format.
/// Accepts a bare number, a Cloud PR URL, or `workspace/repo#id`.
pub fn parse_pull_request_target_bitbucket(input: &str) -> Result<PullRequestTarget> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return malformed_target(input);
    }
    if let Some(target) = parse_numeric_target(trimmed) {
        return Ok(target);
    }
    if let Some(target) = parse_bitbucket_url_target(trimmed) {
        return Ok(target);
    }
    if let Some(target) = parse_bitbucket_repo_hash_target(trimmed) {
        return Ok(target);
    }
    malformed_target(input)
}

/// Parse a Bitbucket Cloud remote URL into a `ForgeRepository`.
///
/// Handles SCP-like (`git@bitbucket.org:ws/repo.git`), HTTPS
/// (`https://user@bitbucket.org/ws/repo.git`), and SSH-scheme
/// (`ssh://git@bitbucket.org/ws/repo.git`) forms.
///
/// Returns `None` for anything that is not `bitbucket.org`, including
/// self-hosted Data Center instances: they expose an unrelated REST 1.0 API
/// this backend cannot drive, so they must not be claimed here.
pub fn parse_bitbucket_remote_url(remote_url: &str) -> Option<ForgeRepository> {
    let trimmed = trim_url_suffix(remote_url.trim());
    if trimmed.is_empty() {
        return None;
    }

    if let Some((host, path)) = parse_scp_like_remote(trimmed) {
        let resolved = resolve_ssh_hostname(host);
        if !is_bitbucket_cloud_host(&resolved) {
            return None;
        }
        return bitbucket_repository_from_path(BITBUCKET_CLOUD_HOST, path);
    }

    let without_scheme = strip_scheme(trimmed)?;
    // HTTPS clone URLs embed the account name: `https://me@bitbucket.org/...`.
    let without_user = without_scheme
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(without_scheme);
    let (host, path) = without_user.split_once('/')?;
    if !is_bitbucket_cloud_host(host) {
        return None;
    }
    bitbucket_repository_from_path(BITBUCKET_CLOUD_HOST, path)
}

/// True only for Bitbucket Cloud. `altssh.bitbucket.org` is the SSH-over-443
/// transport host used on networks that block port 22; it maps back to the
/// same Cloud instance.
fn is_bitbucket_cloud_host(host: &str) -> bool {
    host.eq_ignore_ascii_case(BITBUCKET_CLOUD_HOST)
        || host.eq_ignore_ascii_case("altssh.bitbucket.org")
        || host.eq_ignore_ascii_case("api.bitbucket.org")
}

fn bitbucket_repository_from_path(host: &str, path: &str) -> Option<ForgeRepository> {
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let workspace = parts.next()?;
    let repo = parts.next()?;
    Some(ForgeRepository::bitbucket(
        host,
        workspace,
        strip_git_suffix(trim_url_suffix(repo)),
    ))
}

/// `https://bitbucket.org/{workspace}/{repo}/pull-requests/{id}`
fn parse_bitbucket_url_target(target: &str) -> Option<PullRequestTarget> {
    let without_scheme = strip_scheme(target)?;
    let trimmed = trim_url_suffix(without_scheme);
    let parts: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty()).collect();
    // [host, workspace, repo, "pull-requests", <id>, ..]
    if parts.len() < 5 {
        return None;
    }
    if !is_bitbucket_cloud_host(parts[0]) || parts[3] != "pull-requests" {
        return None;
    }
    let number = parts[4].parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }
    Some(PullRequestTarget::with_repository(
        ForgeRepository::bitbucket(BITBUCKET_CLOUD_HOST, parts[1], strip_git_suffix(parts[2])),
        number,
        target,
    ))
}

/// `workspace/repo#id`, optionally prefixed with the host.
fn parse_bitbucket_repo_hash_target(target: &str) -> Option<PullRequestTarget> {
    let (repo_part, number_part) = target.split_once('#')?;
    let number = number_part.parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }
    let parts: Vec<&str> = repo_part.split('/').filter(|p| !p.is_empty()).collect();
    // Only claim this shape when the host is spelled out; a bare
    // `owner/repo#1` is ambiguous and stays with the GitHub parser.
    let (workspace, repo) = match parts.as_slice() {
        [host, workspace, repo] if is_bitbucket_cloud_host(host) => (*workspace, *repo),
        _ => return None,
    };
    Some(PullRequestTarget::with_repository(
        ForgeRepository::bitbucket(BITBUCKET_CLOUD_HOST, workspace, strip_git_suffix(repo)),
        number,
        target,
    ))
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
            in_block = value.split_whitespace().any(|pat| pat == alias);
        } else if key.eq_ignore_ascii_case("Match") {
            in_block = false;
        } else if in_block && key.eq_ignore_ascii_case("HostName") {
            return value.to_string();
        }
    }
    alias.to_string()
}

fn map_bkt_error(error: BktCommandError) -> TuicrError {
    match error {
        BktCommandError::MissingBkt => TuicrError::Forge(
            "Bitbucket integration requires `bkt`.\n\
             Install it with `brew install avivsinai/tap/bitbucket-cli`, \
             then run `bkt auth login https://bitbucket.org --kind cloud --web-token`."
                .to_string(),
        ),
        BktCommandError::Failed { stderr, .. } if looks_like_auth_failure(&stderr) => {
            TuicrError::Forge(
                "Bitbucket authentication failed.\n\
                 Run `bkt auth status`, then `bkt auth login https://bitbucket.org --kind cloud`."
                    .to_string(),
            )
        }
        BktCommandError::Failed { stderr, status } => TuicrError::Forge(format!(
            "Bitbucket command failed: {}",
            detail(stderr, status)
        )),
    }
}

fn map_create_comment_error(error: BktCommandError) -> TuicrError {
    match &error {
        BktCommandError::Failed { stderr, .. } if looks_like_permission_failure(stderr) => {
            TuicrError::Forge(
                "Bitbucket rejected the comment: your token lacks pull request write access.\n\
                 Re-run `bkt auth login` with the `Pull requests: Write` scope."
                    .to_string(),
            )
        }
        _ => map_bkt_error(error),
    }
}

fn map_approve_error(error: BktCommandError) -> TuicrError {
    match &error {
        BktCommandError::Failed { stderr, .. } if looks_like_permission_failure(stderr) => {
            TuicrError::Forge(
                "Comments were posted, but Bitbucket rejected the approval: \
                 you may not be a reviewer on this pull request."
                    .to_string(),
            )
        }
        _ => map_bkt_error(error),
    }
}

fn detail(stderr: String, status: Option<i32>) -> String {
    if stderr.is_empty() {
        status
            .map(|code| format!("bkt exited with status {code}"))
            .unwrap_or_else(|| "bkt command failed".to_string())
    } else {
        stderr
    }
}

fn looks_like_auth_failure(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("bkt auth login")
        || lower.contains("not logged in")
        || lower.contains("no credentials")
        || lower.contains("authentication failed")
        || lower.contains("requires authentication")
        || lower.contains("401 unauthorized")
}

fn looks_like_permission_failure(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("403 forbidden")
        || lower.contains("http 403")
        || lower.contains("status: 403")
        || lower.contains("not allowed")
        || lower.contains("forbidden")
}

fn malformed_target<T>(input: &str) -> Result<T> {
    Err(TuicrError::Forge(format!(
        "Malformed Bitbucket pull request target: `{input}`"
    )))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::forge::submit::InlineComment;
    use crate::model::FileStatus;

    /// Runner that records every argv it is handed and replays canned
    /// responses in order, so tests assert on the exact commands built
    /// without touching the network.
    struct RecordingRunner {
        calls: RefCell<Vec<Vec<String>>>,
        responses: RefCell<Vec<String>>,
    }

    impl RecordingRunner {
        fn with_responses(responses: Vec<&str>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                responses: RefCell::new(responses.into_iter().map(String::from).collect()),
            }
        }
    }

    impl BktCommandRunner for RecordingRunner {
        fn run(&self, args: &[String]) -> BktCommandResult<String> {
            self.calls.borrow_mut().push(args.to_vec());
            Ok(self
                .responses
                .borrow_mut()
                .drain(..1)
                .next()
                .unwrap_or_default())
        }
    }

    /// Runner that always fails, for error-mapping tests.
    struct FailingRunner {
        stderr: String,
    }

    impl BktCommandRunner for FailingRunner {
        fn run(&self, _args: &[String]) -> BktCommandResult<String> {
            Err(BktCommandError::Failed {
                status: Some(1),
                stderr: self.stderr.clone(),
            })
        }
    }

    fn repo() -> ForgeRepository {
        ForgeRepository::bitbucket("bitbucket.org", "example-workspace", "repo")
    }

    fn backend(responses: Vec<&str>) -> BitbucketBktBackend<RecordingRunner> {
        BitbucketBktBackend::with_runner(Some(repo()), RecordingRunner::with_responses(responses))
    }

    fn details() -> PullRequestDetails {
        PullRequestDetails {
            repository: repo(),
            number: 830,
            title: "A change".to_string(),
            url: "https://bitbucket.org/example-workspace/repo/pull-requests/830".to_string(),
            state: "OPEN".to_string(),
            is_draft: false,
            author: Some("Alice".to_string()),
            head_ref_name: "feature".to_string(),
            base_ref_name: "main".to_string(),
            head_sha: "a".repeat(40),
            base_sha: "b".repeat(40),
            body: String::new(),
            updated_at: None,
            closed: false,
            merged_at: None,
            diff_start_sha: None,
        }
    }

    const PR_JSON: &str = r#"{
      "id": 830, "title": "A change", "state": "OPEN", "draft": false,
      "source": { "branch": { "name": "feature" }, "commit": { "hash": "7d9bf1fa670a" } },
      "destination": { "branch": { "name": "main" }, "commit": { "hash": "b7e0a737bb8c" } },
      "links": { "html": { "href": "https://bitbucket.org/example-workspace/repo/pull-requests/830" } }
    }"#;

    const DIFFSTAT_JSON: &str = r#"{"values":[{
      "status":"modified","old":{"path":"x"},"new":{"path":"x"}
    }]}"#;

    fn inline_comment(line: u32, side: GhSide, start_line: Option<u32>) -> InlineComment {
        InlineComment {
            path: PathBuf::from("src/lib.rs"),
            line,
            side,
            counterpart_line: None,
            start_line,
            start_side: start_line.map(|_| side),
            range_anchors: None,
            old_path: None,
            body: "a note".to_string(),
            comment_id: "local-1".to_string(),
        }
    }

    // ---- command construction -------------------------------------------

    #[test]
    fn should_always_pass_workspace_and_repo_to_first_class_commands() {
        // given — bkt hard-fails when the active context lacks these
        let backend = backend(vec![DIFFSTAT_JSON, "diff --git a/x b/x\n"]);
        // when
        backend.get_pull_request_diff(&details()).unwrap();
        // then
        assert_eq!(
            backend.runner.calls.borrow()[1],
            vec![
                "pr",
                "diff",
                "830",
                "--workspace",
                "example-workspace",
                "--repo",
                "repo"
            ]
        );
    }

    #[test]
    fn should_request_the_page_containing_the_next_unseen_row() {
        // given — two pages already loaded at 30 per page
        let backend = backend(vec![r#"{"values":[]}"#]);
        let query = PullRequestListQuery {
            repository: repo(),
            already_loaded: 60,
            page_size: 30,
            scope: PullRequestListScope::Open,
        };
        // when
        backend.list_pull_requests(query).unwrap();
        // then — Bitbucket pages are 1-based, so row 61 lives on page 3
        assert_eq!(
            backend.runner.calls.borrow()[0],
            vec![
                "api",
                "/2.0/repositories/example-workspace/repo/pullrequests",
                "-P",
                "pagelen=30",
                "-P",
                "page=3",
                "-P",
                "state=OPEN",
            ]
        );
    }

    #[test]
    fn should_report_more_pages_when_next_link_present() {
        // given
        let backend = backend(vec![
            r#"{"values":[],"next":"https://api.bitbucket.org/2.0/x?page=2"}"#,
        ]);
        // when
        let paged = backend
            .list_pull_requests(PullRequestListQuery::first_page(repo(), 30))
            .unwrap();
        // then
        assert!(paged.has_more);
    }

    #[test]
    fn should_filter_by_reviewer_uuid_for_review_requested_scope() {
        // given — the viewer lookup runs first, then the filtered list
        let backend = backend(vec![
            r#"{"uuid":"{viewer-uuid}","username":"","display_name":"Example User"}"#,
            r#"{"values":[]}"#,
        ]);
        // when
        backend
            .list_pull_requests(PullRequestListQuery::first_page_with_scope(
                repo(),
                30,
                PullRequestListScope::ReviewRequested,
            ))
            .unwrap();
        // then
        let calls = backend.runner.calls.borrow();
        assert_eq!(calls[0], vec!["api", "/2.0/user"]);
        // Cloud has no workspace-wide reviewer endpoint; `q` is the
        // equivalent. The state clause must be inside `q` — a standalone
        // `state` param is silently ignored once `q` is present, which leaked
        // merged pull requests into the list.
        assert!(
            calls[1].contains(&"q=state=\"OPEN\" AND reviewers.uuid=\"{viewer-uuid}\"".to_string()),
            "expected state to be folded into the q filter, got {:?}",
            calls[1]
        );
        assert!(
            !calls[1].iter().any(|arg| arg == "state=OPEN"),
            "a standalone state param is ignored by Cloud and must not be sent: {:?}",
            calls[1]
        );
    }

    #[test]
    fn should_use_the_standalone_state_param_for_the_open_scope() {
        // given — without `q`, the plain `state` filter is honoured
        let backend = backend(vec![r#"{"values":[]}"#]);
        // when
        backend
            .list_pull_requests(PullRequestListQuery::first_page(repo(), 30))
            .unwrap();
        // then — and no viewer lookup is needed for this scope
        let calls = backend.runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains(&"state=OPEN".to_string()));
    }

    #[test]
    fn should_fail_clearly_when_viewer_identity_is_unavailable() {
        // given — Cloud returns a user with no uuid
        let backend = backend(vec![r#"{"username":"","display_name":""}"#]);
        // when
        let err = backend
            .list_pull_requests(PullRequestListQuery::first_page_with_scope(
                repo(),
                30,
                PullRequestListScope::ReviewRequested,
            ))
            .unwrap_err();
        // then
        assert!(
            err.to_string().contains("bkt auth status"),
            "unexpected error: {err}"
        );
    }

    // ---- short-SHA promotion --------------------------------------------

    #[test]
    fn should_promote_abbreviated_shas_to_full_length() {
        // given — Cloud reports 12-char hashes on the PR payload
        let full_head = "7d9bf1fa670a02d075ee60b7e2034bced095e096";
        let full_base = "b7e0a737bb8c1111111111111111111111111111";
        let backend = backend(vec![
            PR_JSON,
            &format!(r#"{{"hash":"{full_head}","message":"x"}}"#),
            &format!(r#"{{"hash":"{full_base}","message":"y"}}"#),
        ]);
        // when
        let details = backend
            .get_pull_request(PullRequestTarget::with_repository(repo(), 830, "830"))
            .unwrap();
        // then — a stable PrSessionKey needs the full hash
        assert_eq!(details.head_sha, full_head);
        assert_eq!(details.base_sha, full_base);
        let calls = backend.runner.calls.borrow();
        assert_eq!(
            calls[1][1],
            format!("/2.0/repositories/example-workspace/repo/commit/7d9bf1fa670a")
        );
    }

    #[test]
    fn should_keep_short_sha_when_promotion_returns_a_different_commit() {
        // given — a defensive guard against the API echoing an unrelated hash
        let backend = backend(vec![
            PR_JSON,
            r#"{"hash":"ffffffffffffffffffffffffffffffffffffffff","message":"x"}"#,
            r#"{"hash":"ffffffffffffffffffffffffffffffffffffffff","message":"y"}"#,
        ]);
        // when
        let details = backend
            .get_pull_request(PullRequestTarget::with_repository(repo(), 830, "830"))
            .unwrap();
        // then — the prefix check rejects the mismatch
        assert_eq!(details.head_sha, "7d9bf1fa670a");
    }

    #[test]
    fn should_not_promote_a_sha_that_is_already_full_length() {
        // given — head is already 40 chars, base is empty
        let json = r#"{
          "id": 1, "state": "OPEN",
          "source": { "branch": { "name": "f" },
                      "commit": { "hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" } },
          "destination": { "branch": { "name": "main" } },
          "links": { "html": { "href": "u" } }
        }"#;
        let backend = backend(vec![json]);
        // when
        backend
            .get_pull_request(PullRequestTarget::with_repository(repo(), 1, "1"))
            .unwrap();
        // then — only the PR fetch happened; no commit lookups
        assert_eq!(backend.runner.calls.borrow().len(), 1);
    }

    // ---- commits, threads, context --------------------------------------

    #[test]
    fn should_return_commits_oldest_first() {
        // given — Bitbucket lists newest-first
        let backend = backend(vec![
            r#"{"values":[
              {"hash":"cccccccccccccccccccccccccccccccccccccccc","message":"third"},
              {"hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","message":"second"},
              {"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","message":"first"}
            ]}"#,
        ]);
        // when
        let commits = backend.list_pull_request_commits(&details()).unwrap();
        // then — the trait contract is chronological
        let summaries: Vec<&str> = commits.iter().map(|c| c.summary.as_str()).collect();
        assert_eq!(summaries, vec!["first", "second", "third"]);
    }

    #[test]
    fn should_stop_paginating_when_no_next_link() {
        // given
        let backend = backend(vec![r#"{"values":[{"hash":"a","message":"m"}]}"#]);
        // when
        backend.list_pull_request_commits(&details()).unwrap();
        // then — one page only, not MAX_PAGES
        assert_eq!(backend.runner.calls.borrow().len(), 1);
    }

    #[test]
    fn should_never_send_an_explicit_page_param_when_paginating() {
        // The pull request `commits` endpoint rejects `page` with
        // `400 Invalid page` even for page 1, so pagination must follow the
        // server's `next` URL instead of numbering pages itself.
        let next = "https://api.bitbucket.org/2.0/repositories/example-workspace/repo\
                    /pullrequests/830/commits?pagelen=100&page=2";
        let backend = backend(vec![
            &format!(r#"{{"values":[{{"hash":"a","message":"m"}}],"next":"{next}"}}"#),
            r#"{"values":[{"hash":"b","message":"m2"}]}"#,
        ]);
        // when
        let commits = backend.list_pull_request_commits(&details()).unwrap();
        // then
        let calls = backend.runner.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert!(
            !calls.iter().flatten().any(|arg| arg.starts_with("page=")),
            "must not construct page numbers: {calls:?}"
        );
        // The first request sets pagelen; the second reuses the `next` URL,
        // which already carries it.
        assert!(calls[0].contains(&"pagelen=100".to_string()));
        assert_eq!(calls[1], vec!["api", next]);
        assert_eq!(commits.len(), 2);
    }

    #[test]
    fn should_stop_after_the_page_ceiling_even_if_next_keeps_coming() {
        // given — a server that always advertises another page
        struct AlwaysMore;
        impl BktCommandRunner for AlwaysMore {
            fn run(&self, _args: &[String]) -> BktCommandResult<String> {
                Ok(r#"{"values":[{"hash":"a","message":"m"}],
                       "next":"https://api.bitbucket.org/2.0/next"}"#
                    .to_string())
            }
        }
        let backend = BitbucketBktBackend::with_runner(Some(repo()), AlwaysMore);
        // when
        let commits = backend.list_pull_request_commits(&details()).unwrap();
        // then — bounded, not an infinite loop
        assert_eq!(commits.len(), MAX_PAGES);
    }

    #[test]
    fn should_cap_the_pull_request_list_page_size() {
        // given — Cloud rejects pagelen > 50 on this collection
        let backend = backend(vec![r#"{"values":[]}"#]);
        let query = PullRequestListQuery {
            repository: repo(),
            already_loaded: 0,
            page_size: 500,
            scope: PullRequestListScope::Open,
        };
        // when
        backend.list_pull_requests(query).unwrap();
        // then
        assert!(
            backend.runner.calls.borrow()[0].contains(&"pagelen=50".to_string()),
            "expected the page size to be clamped: {:?}",
            backend.runner.calls.borrow()[0]
        );
    }

    #[test]
    fn should_reverse_the_diff_spec_for_a_commit_range() {
        // given — no local checkout, so it goes through the API
        let backend = backend(vec![DIFFSTAT_JSON, "diff --git a/x b/x\n"]);
        // when
        backend
            .get_pull_request_commit_range_diff(&details(), "oldsha", "newsha")
            .unwrap();
        // then — Bitbucket takes `new..old`, the reverse of git
        assert_eq!(
            backend.runner.calls.borrow()[1][1],
            "/2.0/repositories/example-workspace/repo/diff/newsha..oldsha"
        );
    }

    #[test]
    fn should_split_comments_into_threads_and_summaries() {
        // given — one general comment and one inline comment
        let payload = r#"{"values":[
          {"id":1,"content":{"raw":"LGTM"},"user":{"display_name":"A"},"parent":null},
          {"id":2,"content":{"raw":"nit"},"user":{"display_name":"B"},"parent":null,
           "inline":{"path":"src/lib.rs","to":42}}
        ]}"#;
        // when
        let threads = backend(vec![payload])
            .list_review_threads(&details())
            .unwrap();
        let summaries = backend(vec![payload])
            .list_review_summaries(&details())
            .unwrap();
        // then — each comment lands in exactly one of the two views
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].line, Some(42));
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].body, "LGTM");
    }

    #[test]
    fn should_fetch_file_lines_from_the_src_endpoint() {
        // given — no local checkout
        let backend = backend(vec!["one\ntwo\nthree\nfour\n"]);
        let request = ForgeFileLinesRequest {
            repository: repo(),
            base_sha: "b".repeat(40),
            head_sha: "a".repeat(40),
            path: PathBuf::from("src/lib.rs"),
            status: FileStatus::Modified,
            side: crate::forge::traits::ForgeFileSide::Head,
            start_line: 2,
            end_line: 3,
        };
        // when
        let lines = backend.fetch_file_lines(request).unwrap();
        // then
        assert_eq!(
            backend.runner.calls.borrow()[0][1],
            format!(
                "/2.0/repositories/example-workspace/repo/src/{}/src/lib.rs",
                "a".repeat(40)
            )
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].content, "two");
        assert_eq!(lines[1].content, "three");
    }

    // ---- review submission ----------------------------------------------

    #[test]
    fn should_post_body_then_inline_comments_on_submit() {
        // given
        let backend = backend(vec![r#"{"id":111}"#, r#"{"id":222}"#]);
        let comments = vec![inline_comment(42, GhSide::Right, None)];
        let request = CreateReviewRequest {
            event: SubmitEvent::Comment,
            commit_id: &"a".repeat(40),
            body: "overall looks fine",
            comments: &comments,
        };
        // when
        let response = backend.create_review(&details(), request).unwrap();
        // then — general comment first, then the inline one
        let calls = backend.runner.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0][2..4], ["--method".to_string(), "POST".to_string()]);
        let general: serde_json::Value = serde_json::from_str(&calls[0][5]).unwrap();
        assert_eq!(general["content"]["raw"], "overall looks fine");
        assert!(general.get("inline").is_none());
        let inline: serde_json::Value = serde_json::from_str(&calls[1][5]).unwrap();
        assert_eq!(inline["inline"]["path"], "src/lib.rs");
        assert_eq!(inline["inline"]["to"], 42);
        // The first created comment id is reported back for lifecycle writes.
        assert_eq!(response.id, 111);
        assert_eq!(response.state, "COMMENTED");
    }

    #[test]
    fn should_skip_the_general_comment_when_body_is_empty() {
        // given
        let backend = backend(vec![r#"{"id":222}"#]);
        let comments = vec![inline_comment(1, GhSide::Right, None)];
        let request = CreateReviewRequest {
            event: SubmitEvent::Comment,
            commit_id: &"a".repeat(40),
            body: "",
            comments: &comments,
        };
        // when
        backend.create_review(&details(), request).unwrap();
        // then — only the inline comment is posted
        assert_eq!(backend.runner.calls.borrow().len(), 1);
    }

    #[test]
    fn should_anchor_left_side_comments_with_from() {
        // given — a comment on a deleted line
        let backend = backend(vec![r#"{"id":1}"#]);
        let comments = vec![inline_comment(17, GhSide::Left, None)];
        let request = CreateReviewRequest {
            event: SubmitEvent::Comment,
            commit_id: &"a".repeat(40),
            body: "",
            comments: &comments,
        };
        // when
        backend.create_review(&details(), request).unwrap();
        // then
        let calls = backend.runner.calls.borrow();
        let posted: serde_json::Value = serde_json::from_str(&calls[0][5]).unwrap();
        assert_eq!(posted["inline"]["from"], 17);
        assert!(posted["inline"].get("to").is_none());
    }

    #[test]
    fn should_send_a_range_anchor_for_multi_line_comments() {
        // given — a selection spanning lines 10..14 on the head side
        let backend = backend(vec![r#"{"id":1}"#]);
        let comments = vec![inline_comment(14, GhSide::Right, Some(10))];
        let request = CreateReviewRequest {
            event: SubmitEvent::Comment,
            commit_id: &"a".repeat(40),
            body: "",
            comments: &comments,
        };
        // when
        backend.create_review(&details(), request).unwrap();
        // then — Cloud expresses ranges through start_to/start_from
        let calls = backend.runner.calls.borrow();
        let posted: serde_json::Value = serde_json::from_str(&calls[0][5]).unwrap();
        assert_eq!(posted["inline"]["to"], 14);
        assert_eq!(posted["inline"]["start_to"], 10);
    }

    #[test]
    fn should_omit_range_anchor_when_start_equals_end() {
        // given — a single-line selection reported with an explicit start
        let backend = backend(vec![r#"{"id":1}"#]);
        let comments = vec![inline_comment(9, GhSide::Right, Some(9))];
        let request = CreateReviewRequest {
            event: SubmitEvent::Comment,
            commit_id: &"a".repeat(40),
            body: "",
            comments: &comments,
        };
        // when
        backend.create_review(&details(), request).unwrap();
        // then
        let calls = backend.runner.calls.borrow();
        let posted: serde_json::Value = serde_json::from_str(&calls[0][5]).unwrap();
        assert!(posted["inline"].get("start_to").is_none());
    }

    #[test]
    fn should_approve_after_posting_comments() {
        // given
        let backend = backend(vec![r#"{"id":1}"#, "{}"]);
        let comments = vec![inline_comment(3, GhSide::Right, None)];
        let request = CreateReviewRequest {
            event: SubmitEvent::Approve,
            commit_id: &"a".repeat(40),
            body: "",
            comments: &comments,
        };
        // when
        let response = backend.create_review(&details(), request).unwrap();
        // then — comments land before the approval, so a failed approve
        // doesn't lose the feedback
        let calls = backend.runner.calls.borrow();
        assert_eq!(calls[0][0], "api");
        assert_eq!(
            calls[1],
            vec![
                "pr",
                "approve",
                "830",
                "--workspace",
                "example-workspace",
                "--repo",
                "repo"
            ]
        );
        assert_eq!(response.state, "APPROVED");
    }

    #[test]
    fn should_synthesize_a_response_when_the_comment_id_is_unparseable() {
        // given — Bitbucket has no review object to report
        let backend = backend(vec!["not json"]);
        let request = CreateReviewRequest {
            event: SubmitEvent::Comment,
            commit_id: &"a".repeat(40),
            body: "hello",
            comments: &[],
        };
        // when
        let response = backend.create_review(&details(), request).unwrap();
        // then — the comment still posted; only the id is unknown
        assert_eq!(response.id, 0);
        assert_eq!(
            response.html_url,
            "https://bitbucket.org/example-workspace/repo/pull-requests/830"
        );
    }

    #[test]
    fn should_reject_request_changes_and_draft_before_posting_anything() {
        for event in [SubmitEvent::RequestChanges, SubmitEvent::Draft] {
            // given
            let backend = backend(vec![]);
            let request = CreateReviewRequest {
                event,
                commit_id: &"a".repeat(40),
                body: "text",
                comments: &[],
            };
            // when
            let err = backend.create_review(&details(), request).unwrap_err();
            // then — nothing was sent, so the user can retry with `:submit`
            assert!(
                err.to_string().contains("not supported for Bitbucket yet"),
                "unexpected error for {event:?}: {err}"
            );
            assert!(backend.runner.calls.borrow().is_empty());
        }
    }

    // ---- error mapping ---------------------------------------------------

    #[test]
    fn should_explain_how_to_authenticate_on_auth_failure() {
        // given
        let backend = BitbucketBktBackend::with_runner(
            Some(repo()),
            FailingRunner {
                stderr: "401 Unauthorized".to_string(),
            },
        );
        // when
        let err = backend.get_pull_request_diff(&details()).unwrap_err();
        // then
        assert!(
            err.to_string().contains("bkt auth login"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn should_explain_missing_scope_when_a_comment_is_forbidden() {
        // given
        let backend = BitbucketBktBackend::with_runner(
            Some(repo()),
            FailingRunner {
                stderr: "403 Forbidden".to_string(),
            },
        );
        let request = CreateReviewRequest {
            event: SubmitEvent::Comment,
            commit_id: &"a".repeat(40),
            body: "text",
            comments: &[],
        };
        // when
        let err = backend.create_review(&details(), request).unwrap_err();
        // then
        assert!(
            err.to_string().contains("Pull requests: Write"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn should_say_comments_survived_when_only_the_approval_fails() {
        // given — the approve call fails after comments posted
        struct ApproveFails;
        impl BktCommandRunner for ApproveFails {
            fn run(&self, args: &[String]) -> BktCommandResult<String> {
                if args[0] == "pr" && args[1] == "approve" {
                    return Err(BktCommandError::Failed {
                        status: Some(1),
                        stderr: "403 Forbidden".to_string(),
                    });
                }
                Ok(r#"{"id":1}"#.to_string())
            }
        }
        let backend = BitbucketBktBackend::with_runner(Some(repo()), ApproveFails);
        let request = CreateReviewRequest {
            event: SubmitEvent::Approve,
            commit_id: &"a".repeat(40),
            body: "text",
            comments: &[],
        };
        // when
        let err = backend.create_review(&details(), request).unwrap_err();
        // then — the message must not imply the comments were lost
        assert!(
            err.to_string().contains("Comments were posted"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn should_require_a_repository_for_a_bare_numeric_target() {
        // given — no default repository configured
        let backend =
            BitbucketBktBackend::with_runner(None, RecordingRunner::with_responses(vec![]));
        // when
        let err = backend
            .get_pull_request(PullRequestTarget::number(5, "5"))
            .unwrap_err();
        // then
        assert!(
            err.to_string().contains("does not include a repository"),
            "unexpected error: {err}"
        );
    }

    // ---- remote URL parsing ---------------------------------------------

    #[test]
    fn should_parse_bitbucket_cloud_remote_urls() {
        let expected = ForgeRepository::bitbucket("bitbucket.org", "example-workspace", "repo");
        for url in [
            "https://bitbucket.org/example-workspace/repo.git",
            "https://bitbucket.org/example-workspace/repo",
            // HTTPS clone URLs embed the account name.
            "https://someuser@bitbucket.org/example-workspace/repo.git",
            "git@bitbucket.org:example-workspace/repo.git",
            "ssh://git@bitbucket.org/example-workspace/repo.git",
            // SSH-over-443 transport host maps back to Cloud.
            "git@altssh.bitbucket.org:example-workspace/repo.git",
        ] {
            assert_eq!(
                parse_bitbucket_remote_url(url).as_ref(),
                Some(&expected),
                "failed for {url}"
            );
        }
    }

    #[test]
    fn should_not_claim_non_bitbucket_remotes() {
        for url in [
            "https://github.com/owner/repo.git",
            "git@gitlab.com:owner/repo.git",
            "https://gitlab.example.com/owner/repo.git",
        ] {
            assert!(
                parse_bitbucket_remote_url(url).is_none(),
                "wrongly claimed {url}"
            );
        }
    }

    #[test]
    fn should_not_claim_bitbucket_data_center_remotes() {
        // given — DC speaks REST 1.0, which this backend cannot drive, so it
        // must fall through to the other parsers rather than fail at runtime
        for url in [
            "https://bitbucket.example.com/scm/proj/repo.git",
            "git@bitbucket.mycompany.io:proj/repo.git",
        ] {
            assert!(
                parse_bitbucket_remote_url(url).is_none(),
                "wrongly claimed DC remote {url}"
            );
        }
    }

    // ---- PR target parsing ----------------------------------------------

    #[test]
    fn should_parse_a_bitbucket_pull_request_url() {
        // given / when
        let target = parse_pull_request_target_bitbucket(
            "https://bitbucket.org/example-workspace/repo/pull-requests/830",
        )
        .unwrap();
        // then
        assert_eq!(target.number, 830);
        assert_eq!(
            target.repository,
            Some(ForgeRepository::bitbucket(
                "bitbucket.org",
                "example-workspace",
                "repo"
            ))
        );
    }

    #[test]
    fn should_parse_a_pull_request_url_with_trailing_segments() {
        // given — Bitbucket appends view state to the path
        let target = parse_pull_request_target_bitbucket(
            "https://bitbucket.org/example-workspace/repo/pull-requests/830/some-branch-name/diff",
        )
        .unwrap();
        // then
        assert_eq!(target.number, 830);
    }

    #[test]
    fn should_parse_a_bare_number() {
        // given / when
        let target = parse_pull_request_target_bitbucket("42").unwrap();
        // then — the repository is resolved from the checkout later
        assert_eq!(target.number, 42);
        assert!(target.repository.is_none());
    }

    #[test]
    fn should_parse_a_host_qualified_repo_hash_target() {
        // given / when
        let target =
            parse_pull_request_target_bitbucket("bitbucket.org/example-workspace/repo#7").unwrap();
        // then
        assert_eq!(target.number, 7);
        assert_eq!(
            target.repository.map(|r| r.slug()),
            Some("example-workspace/repo".to_string())
        );
    }

    /// Opt-in end-to-end check against a real Bitbucket Cloud pull request.
    ///
    /// Requires `bkt` to be installed and authenticated. Point it at a PR you
    /// can read, then run:
    ///
    /// ```text
    /// TUICR_BB_WORKSPACE=myteam TUICR_BB_REPO=my-service TUICR_BB_PR=42 \
    ///   cargo test bitbucket_live -- --ignored --nocapture
    /// ```
    ///
    /// Read-only: it never posts a comment or approves anything.
    #[test]
    #[ignore = "hits the live Bitbucket API; needs TUICR_BB_* env vars"]
    fn bitbucket_live_read_path() {
        let (Ok(workspace), Ok(name), Ok(number)) = (
            std::env::var("TUICR_BB_WORKSPACE"),
            std::env::var("TUICR_BB_REPO"),
            std::env::var("TUICR_BB_PR"),
        ) else {
            panic!("set TUICR_BB_WORKSPACE, TUICR_BB_REPO and TUICR_BB_PR");
        };
        let number: u64 = number.parse().expect("TUICR_BB_PR must be a number");
        let repository = ForgeRepository::bitbucket(BITBUCKET_CLOUD_HOST, workspace, name);
        let backend = BitbucketBktBackend::new(Some(repository.clone()));

        let details = backend
            .get_pull_request(PullRequestTarget::with_repository(
                repository.clone(),
                number,
                number.to_string(),
            ))
            .expect("get_pull_request");
        println!(
            "PR #{}: {} [{}] {} -> {}",
            details.number,
            details.title,
            details.state,
            details.head_ref_name,
            details.base_ref_name
        );
        // The promotion step must have widened Cloud's 12-char hashes.
        assert_eq!(details.head_sha.len(), 40, "head_sha was not promoted");
        assert_eq!(details.number, number);

        let patches = backend
            .get_pull_request_diff(&details)
            .expect("get_pull_request_diff");
        assert!(
            patches
                .iter()
                .any(|patch| patch.patch.contains("diff --git ")),
            "structured patches are missing Git bodies"
        );

        let commits = backend
            .list_pull_request_commits(&details)
            .expect("list_pull_request_commits");
        println!("{} commits", commits.len());
        // Every commit oid must match head_sha's width so commit scoping works.
        for commit in &commits {
            assert_eq!(commit.oid.len(), 40, "short oid in commit list");
        }

        let threads = backend
            .list_review_threads(&details)
            .expect("list_review_threads");
        for thread in &threads {
            println!(
                "thread {} {}:{:?} resolved={} outdated={} comments={}",
                thread.id,
                thread.path,
                thread.line,
                thread.is_resolved,
                thread.is_outdated,
                thread.comments.len()
            );
            assert!(thread.line.is_some(), "inline thread without an anchor");
        }

        let summaries = backend
            .list_review_summaries(&details)
            .expect("list_review_summaries");
        println!("{} general comments", summaries.len());

        let metadata = backend
            .list_pull_request_review_metadata(&details)
            .expect("list_pull_request_review_metadata");
        println!(
            "viewer={:?} reviews={}",
            metadata.viewer_login,
            metadata.reviews.len()
        );

        // The PR-tab entry point, in both scopes.
        for scope in [
            PullRequestListScope::Open,
            PullRequestListScope::ReviewRequested,
        ] {
            let listed = backend
                .list_pull_requests(PullRequestListQuery::first_page_with_scope(
                    repository.clone(),
                    30,
                    scope,
                ))
                .unwrap_or_else(|err| panic!("list_pull_requests({scope:?}): {err}"));
            println!(
                "{:?}: {} rows, has_more={}",
                scope,
                listed.pull_requests.len(),
                listed.has_more
            );
            for row in listed.pull_requests.iter().take(3) {
                println!("  #{} {} [{}]", row.number, row.title, row.state);
            }
        }
    }

    #[test]
    fn should_not_claim_ambiguous_or_foreign_targets() {
        // A bare `owner/repo#N` is ambiguous across forges and must fall
        // through to the GitHub parser; GitHub/GitLab URLs are not ours.
        for target in [
            "owner/repo#5",
            "https://github.com/owner/repo/pull/5",
            "https://gitlab.com/owner/repo/-/merge_requests/5",
            "",
            "not-a-target",
            "0",
        ] {
            assert!(
                parse_pull_request_target_bitbucket(target).is_err(),
                "wrongly claimed {target:?}"
            );
        }
    }
}
