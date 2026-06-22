use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use sha1::{Digest, Sha1};

use crate::error::{Result, TuicrError};
use crate::forge::remote_comments::RemoteReviewThread;
use crate::forge::traits::{
    ForgeBackend, ForgeFileLinesRequest, ForgeRepository, GhCreateReviewResponse,
    PagedPullRequests, PullRequestCommit, PullRequestDetails, PullRequestListQuery,
    PullRequestListScope, PullRequestTarget,
};
use crate::model::{DiffLine, LineOrigin};
use crate::process::{
    CommandOutputError, CommandOutputErrorKind, run_command_output, run_command_output_with_stdin,
};

use super::models::{GlabCommit, GlabDiscussion, GlabMrDetails, GlabMrSummary};
use crate::forge::submit::{GhSide, SubmitEvent};
use crate::forge::traits::CreateReviewRequest;

const DEFAULT_GITLAB_HOST: &str = "gitlab.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlabCommandError {
    MissingGlab,
    Failed { status: Option<i32>, stderr: String },
}

pub type GlabCommandResult<T> = std::result::Result<T, GlabCommandError>;

pub trait GlabCommandRunner {
    fn run(&self, args: &[String]) -> GlabCommandResult<String>;

    fn run_with_stdin(&self, _args: &[String], _stdin: &str) -> GlabCommandResult<String> {
        panic!("run_with_stdin not implemented for this GlabCommandRunner");
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemGlabRunner;

impl GlabCommandRunner for SystemGlabRunner {
    fn run(&self, args: &[String]) -> GlabCommandResult<String> {
        run_command_output(
            "glab",
            None,
            args.iter().map(|arg| OsStr::new(arg.as_str())),
        )
        .map_err(GlabCommandError::from)
    }

    fn run_with_stdin(&self, args: &[String], stdin: &str) -> GlabCommandResult<String> {
        run_command_output_with_stdin(
            "glab",
            None,
            args.iter().map(|arg| OsStr::new(arg.as_str())),
            stdin,
        )
        .map_err(GlabCommandError::from)
    }
}

impl From<CommandOutputError> for GlabCommandError {
    fn from(error: CommandOutputError) -> Self {
        match error.kind {
            CommandOutputErrorKind::NotFound => Self::MissingGlab,
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

/// Percent-encode `owner/repo` as `owner%2Frepo` for GitLab project API paths.
fn gl_project_path(owner: &str, name: &str) -> String {
    format!("{}/{}", owner, name).replace('/', "%2F")
}

/// Percent-encode a file path for use in GitLab repository file API endpoints.
fn gl_encode_file_path(path: &str) -> String {
    path.replace('/', "%2F")
        .replace(' ', "%20")
        .replace('#', "%23")
        .replace('?', "%3F")
}

#[derive(Debug, Clone)]
pub struct GitLabGlabBackend<R = SystemGlabRunner> {
    default_repository: Option<ForgeRepository>,
    runner: R,
    local_checkout: Option<PathBuf>,
}

impl GitLabGlabBackend<SystemGlabRunner> {
    pub fn new(default_repository: Option<ForgeRepository>) -> Self {
        Self {
            default_repository,
            runner: SystemGlabRunner,
            local_checkout: None,
        }
    }

    pub fn with_local_checkout(mut self, checkout: Option<PathBuf>) -> Self {
        self.local_checkout = checkout;
        self
    }
}

impl<R> GitLabGlabBackend<R>
where
    R: GlabCommandRunner,
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
                    "GitLab merge request target `{}` does not include a repository",
                    target.original
                ))
            })
    }

    fn run_glab(&self, args: Vec<String>, host: &str) -> Result<String> {
        self.runner
            .run(&args)
            .map_err(|err| map_glab_error(err, host))
    }

    /// Build the value for `glab`'s `--repo` flag. For non-default hosts we
    /// pass a full URL so `glab mr <subcommand>` resolves the right instance —
    /// those subcommands don't accept a `--hostname` flag, but `--repo` does
    /// accept `OWNER/REPO`, the full URL, or a git URL.
    fn repo_arg(repo: &ForgeRepository) -> String {
        if repo.host == DEFAULT_GITLAB_HOST {
            repo.slug()
        } else {
            format!("https://{}/{}", repo.host, repo.slug())
        }
    }

    /// Extra args to select a specific GitLab instance for `glab api` calls.
    /// Only `glab api` (and `glab auth`) accept `--hostname`; the `mr`
    /// subcommands don't, so use `repo_arg` for those.
    fn api_hostname_args(repo: &ForgeRepository) -> Vec<String> {
        if repo.host != DEFAULT_GITLAB_HOST {
            vec!["--hostname".to_string(), repo.host.clone()]
        } else {
            vec![]
        }
    }
}

impl<R> ForgeBackend for GitLabGlabBackend<R>
where
    R: GlabCommandRunner,
{
    fn list_pull_requests(&self, query: PullRequestListQuery) -> Result<PagedPullRequests> {
        let page_size = query.page_size.max(1);
        let requested = query.already_loaded + page_size + 1;
        let mut args = vec![
            "mr".to_string(),
            "list".to_string(),
            "--repo".to_string(),
            Self::repo_arg(&query.repository),
            "--output".to_string(),
            "json".to_string(),
            "--per-page".to_string(),
            requested.to_string(),
        ];
        if query.scope == PullRequestListScope::ReviewRequested {
            args.push("--reviewer=@me".to_string());
        }
        let output = self.run_glab(args, &query.repository.host)?;
        let rows: Vec<GlabMrSummary> = serde_json::from_str(&output)?;
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
        let args = vec![
            "mr".to_string(),
            "view".to_string(),
            target.number.to_string(),
            "--repo".to_string(),
            Self::repo_arg(&repository),
            "--output".to_string(),
            "json".to_string(),
        ];
        let output = self.run_glab(args, &repository.host)?;
        let mr: GlabMrDetails = serde_json::from_str(&output)?;
        mr.into_details(&repository)
    }

    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<String> {
        let args = vec![
            "mr".to_string(),
            "diff".to_string(),
            pr.number.to_string(),
            "--repo".to_string(),
            Self::repo_arg(&pr.repository),
            "--color=never".to_string(),
        ];
        let raw = self.run_glab(args, &pr.repository.host)?;
        Ok(inject_git_diff_headers(&raw))
    }

    fn local_checkout_path(&self) -> Option<PathBuf> {
        self.local_checkout.clone()
    }

    fn list_pull_request_commits(&self, pr: &PullRequestDetails) -> Result<Vec<PullRequestCommit>> {
        let project = gl_project_path(&pr.repository.owner, &pr.repository.name);
        let mut commits: Vec<PullRequestCommit> = Vec::new();
        for page in 1..=10 {
            let endpoint = format!(
                "projects/{}/merge_requests/{}/commits?per_page=100&page={}",
                project, pr.number, page,
            );
            let mut args = vec!["api".to_string()];
            args.extend(Self::api_hostname_args(&pr.repository));
            args.push(endpoint);
            let output = self.run_glab(args, &pr.repository.host)?;
            let rows: Vec<GlabCommit> = serde_json::from_str(&output)?;
            let received = rows.len();
            commits.extend(rows.into_iter().map(GlabCommit::into_pull_request_commit));
            if received < 100 {
                break;
            }
        }
        Ok(commits)
    }

    fn get_pull_request_commit_range_diff(
        &self,
        _pr: &PullRequestDetails,
        start_sha: &str,
        end_sha: &str,
    ) -> Result<String> {
        if let Some(root) = self.local_checkout.as_deref()
            && let Some(diff) = local_range_diff(root, start_sha, end_sha)
        {
            return Ok(diff);
        }
        Err(TuicrError::UnsupportedOperation(
            "Commit range diff without local checkout not yet supported for GitLab".to_string(),
        ))
    }

    fn list_review_threads(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewThread>> {
        let project = gl_project_path(&pr.repository.owner, &pr.repository.name);
        let mut all: Vec<RemoteReviewThread> = Vec::new();
        for page in 1..=100 {
            let endpoint = format!(
                "projects/{}/merge_requests/{}/discussions?per_page=100&page={}",
                project, pr.number, page,
            );
            let mut args = vec!["api".to_string()];
            args.extend(Self::api_hostname_args(&pr.repository));
            args.push(endpoint);
            let output = self.run_glab(args, &pr.repository.host)?;
            if std::env::var("TUICR_GLAB_DEBUG").is_ok() {
                glab_debug_log(&format!(
                    "[GLAB_DEBUG] list_review_threads page={page} response: {output}\n"
                ));
            }
            let discussions: Vec<GlabDiscussion> = serde_json::from_str(&output)?;
            let received = discussions.len();
            if std::env::var("TUICR_GLAB_DEBUG").is_ok() {
                let positional = discussions
                    .iter()
                    .filter(|d| d.notes.first().and_then(|n| n.position.as_ref()).is_some())
                    .count();
                glab_debug_log(&format!(
                    "[GLAB_DEBUG] list_review_threads page={page}: {received} discussions, {positional} with position\n"
                ));
            }
            all.extend(
                discussions
                    .into_iter()
                    .filter_map(|d| d.into_review_thread()),
            );
            if received < 100 {
                break;
            }
        }
        if std::env::var("TUICR_GLAB_DEBUG").is_ok() {
            glab_debug_log(&format!(
                "[GLAB_DEBUG] list_review_threads total inline threads returned: {}\n",
                all.len()
            ));
        }
        Ok(all)
    }

    fn fetch_file_lines(&self, request: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
        if request.start_line == 0 || request.start_line > request.end_line {
            return Ok(Vec::new());
        }

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

    fn file_line_count(&self, request: ForgeFileLinesRequest) -> Result<u32> {
        let local_content = self
            .local_checkout
            .as_deref()
            .and_then(|root| read_blob_with_repo(root, request.sha(), request.path.as_path()));
        let content = if let Some(content) = local_content {
            content
        } else {
            self.fetch_file_via_api(&request)?
        };
        Ok(content.lines().count() as u32)
    }

    fn create_review(
        &self,
        pr: &PullRequestDetails,
        request: CreateReviewRequest<'_>,
    ) -> Result<GhCreateReviewResponse> {
        match request.event {
            SubmitEvent::Comment
            | SubmitEvent::Approve
            | SubmitEvent::RequestChanges
            | SubmitEvent::Draft => {}
        }

        // Draft submissions create GitLab draft notes (the pending-review
        // primitive) and stop short of publishing, mirroring GitHub's pending
        // review. The author publishes from the GitLab "Submit review" UI.
        let is_draft = request.event == SubmitEvent::Draft;

        let project = gl_project_path(&pr.repository.owner, &pr.repository.name);
        let start_sha = pr
            .diff_start_sha
            .as_deref()
            .unwrap_or(&pr.base_sha)
            .to_string();

        let mut first_discussion_id: Option<String> = None;

        // Post the overall review body as a general MR note (if non-empty).
        if !request.body.is_empty() {
            let endpoint = if is_draft {
                format!(
                    "projects/{}/merge_requests/{}/draft_notes",
                    project, pr.number
                )
            } else {
                format!("projects/{}/merge_requests/{}/notes", project, pr.number)
            };
            let body_json = if is_draft {
                serde_json::to_string(&serde_json::json!({ "note": request.body }))?
            } else {
                serde_json::to_string(&serde_json::json!({ "body": request.body }))?
            };
            let mut args = vec![
                "api".to_string(),
                endpoint,
                "--method".to_string(),
                "POST".to_string(),
                "--header".to_string(),
                "Content-Type: application/json".to_string(),
                "--input".to_string(),
                "-".to_string(),
            ];
            args.extend(Self::api_hostname_args(&pr.repository));
            self.runner
                .run_with_stdin(&args, &body_json)
                .map_err(|err| map_create_notes_error(err, &pr.repository.host))?;
        }

        for comment in request.comments {
            let new_path = comment.path.to_string_lossy().replace('\\', "/");
            // GitLab positions need both old_path and new_path. Renamed files
            // set comment.old_path to the base-side path; otherwise both
            // sides share the display path.
            let old_path = comment
                .old_path
                .as_ref()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| new_path.clone());

            // Build the position object with integer line numbers.
            // For context lines (unchanged), GitLab requires BOTH old_line and
            // new_line to resolve the position. For added lines only new_line,
            // for deleted lines only old_line. counterpart_line carries the
            // "other side" line number populated by the diff mapper for context lines.
            let mut position = serde_json::json!({
                "position_type": "text",
                "base_sha": pr.base_sha,
                "start_sha": start_sha,
                "head_sha": pr.head_sha,
                "old_path": old_path,
                "new_path": new_path,
            });
            match comment.side {
                GhSide::Right => {
                    position["new_line"] = serde_json::Value::Number(comment.line.into());
                    if let Some(old_line) = comment.counterpart_line {
                        position["old_line"] = serde_json::Value::Number(old_line.into());
                    }
                }
                GhSide::Left => {
                    position["old_line"] = serde_json::Value::Number(comment.line.into());
                    if let Some(new_line) = comment.counterpart_line {
                        position["new_line"] = serde_json::Value::Number(new_line.into());
                    }
                }
            }
            // Multi-line range comments need an explicit `line_range` so
            // GitLab anchors the discussion across the full selection
            // instead of collapsing to the end line.
            if let Some(start_line) = comment.start_line {
                let start_side = comment.start_side.unwrap_or(comment.side);
                let start_endpoint = gl_range_endpoint(&new_path, start_side, start_line);
                let end_endpoint = gl_range_endpoint(&new_path, comment.side, comment.line);
                position["line_range"] = serde_json::json!({
                    "start": start_endpoint,
                    "end": end_endpoint,
                });
            }
            let body_json = if is_draft {
                serde_json::to_string(&serde_json::json!({
                    "note": comment.body,
                    "position": position,
                }))?
            } else {
                serde_json::to_string(&serde_json::json!({
                    "body": comment.body,
                    "position": position,
                }))?
            };

            let endpoint = if is_draft {
                format!(
                    "projects/{}/merge_requests/{}/draft_notes",
                    project, pr.number,
                )
            } else {
                format!(
                    "projects/{}/merge_requests/{}/discussions",
                    project, pr.number,
                )
            };
            let mut args = vec![
                "api".to_string(),
                endpoint,
                "--method".to_string(),
                "POST".to_string(),
                "--header".to_string(),
                "Content-Type: application/json".to_string(),
                "--input".to_string(),
                "-".to_string(),
            ];
            args.extend(Self::api_hostname_args(&pr.repository));

            if std::env::var("TUICR_GLAB_DEBUG").is_ok() {
                let host = &pr.repository.host;
                let project_encoded =
                    format!("{}/{}", pr.repository.owner, pr.repository.name).replace('/', "%2F");
                let endpoint_label = if is_draft {
                    "draft_notes"
                } else {
                    "discussions"
                };
                let url = format!(
                    "https://{host}/api/v4/projects/{project_encoded}/merge_requests/{}/{endpoint_label}",
                    pr.number
                );
                glab_debug_log(&format!(
                    "[GLAB_DEBUG] equivalent curl command:\n  curl --request POST \\\n    --header 'PRIVATE-TOKEN: <token>' \\\n    --header 'Content-Type: application/json' \\\n    --data '{}' \\\n    '{}'\n  # Get token with: glab auth status -t\n",
                    body_json, url
                ));
            }

            let output = self
                .runner
                .run_with_stdin(&args, &body_json)
                .map_err(|err| map_create_notes_error(err, &pr.repository.host))?;

            if std::env::var("TUICR_GLAB_DEBUG").is_ok() {
                glab_debug_log(&format!("[GLAB_DEBUG] glab response: {output}\n"));
            }

            // Discussions return a string `id`; draft_notes return a numeric
            // one. Capture either so the success message reports a real id.
            if first_discussion_id.is_none()
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&output)
                && let Some(id) = value.get("id").and_then(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .or_else(|| v.as_u64().map(|n| n.to_string()))
                })
            {
                first_discussion_id = Some(id);
            }
        }

        if request.event == SubmitEvent::Approve {
            let endpoint = format!("projects/{}/merge_requests/{}/approve", project, pr.number,);
            let mut args = vec![
                "api".to_string(),
                endpoint,
                "--method".to_string(),
                "POST".to_string(),
            ];
            args.extend(Self::api_hostname_args(&pr.repository));
            self.run_glab(args, &pr.repository.host)?;
        }

        if request.event == SubmitEvent::RequestChanges {
            let full_path = format!("{}/{}", pr.repository.owner, pr.repository.name);
            let query = "mutation($projectPath: ID!, $iid: String!) { \
                mergeRequestRequestChanges(input: { projectPath: $projectPath, iid: $iid }) { errors } }";
            let mut args = vec![
                "api".to_string(),
                "graphql".to_string(),
                "-f".to_string(),
                format!("query={query}"),
                "-f".to_string(),
                format!("projectPath={full_path}"),
                "-f".to_string(),
                format!("iid={}", pr.number),
            ];
            args.extend(Self::api_hostname_args(&pr.repository));
            let output = self.run_glab(args, &pr.repository.host)?;
            check_graphql_errors(&output, "mergeRequestRequestChanges")?;
        }

        let id = first_discussion_id
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let state = if request.event == SubmitEvent::RequestChanges {
            "CHANGES_REQUESTED"
        } else if is_draft {
            "PENDING"
        } else {
            "COMMENTED"
        };

        Ok(GhCreateReviewResponse {
            id,
            html_url: pr.url.clone(),
            state: state.to_string(),
        })
    }
}

impl<R> GitLabGlabBackend<R>
where
    R: GlabCommandRunner,
{
    fn fetch_file_via_api(&self, request: &ForgeFileLinesRequest) -> Result<String> {
        let project = gl_project_path(&request.repository.owner, &request.repository.name);
        let path_str = request.path.to_string_lossy().replace('\\', "/");
        let encoded_path = gl_encode_file_path(&path_str);
        let endpoint = format!(
            "projects/{}/repository/files/{}/raw?ref={}",
            project,
            encoded_path,
            request.sha(),
        );
        let mut args = vec!["api".to_string()];
        args.extend(Self::api_hostname_args(&request.repository));
        args.push(endpoint);
        self.run_glab(args, &request.repository.host)
    }
}

fn slice_to_diff_lines(content: &str, start_line: u32, end_line: u32) -> Vec<DiffLine> {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    for line_num in start_line..=end_line {
        let idx = (line_num - 1) as usize;
        if idx >= lines.len() {
            break;
        }
        result.push(DiffLine {
            origin: LineOrigin::Context,
            content: lines[idx].to_string(),
            old_lineno: Some(line_num),
            new_lineno: Some(line_num),
            highlighted_spans: None,
        });
    }
    result
}

/// Parse a pull request target string in GitLab format.
/// Accepts: numeric IID, full GitLab MR URL, or `owner/repo#iid` / `host/owner/repo#iid`.
pub fn parse_pull_request_target_gitlab(input: &str) -> Result<PullRequestTarget> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return malformed_target(input);
    }

    if let Some(target) = parse_numeric_target(trimmed) {
        return Ok(target);
    }
    if let Some(target) = parse_gitlab_url_target(trimmed) {
        return Ok(target);
    }
    if let Some(target) = parse_gitlab_repo_hash_target(trimmed) {
        return Ok(target);
    }

    malformed_target(input)
}

/// Parse a GitLab remote URL into a `ForgeRepository`.
///
/// Handles SCP-like (`git@gitlab.com:owner/repo.git`), HTTPS
/// (`https://gitlab.com/owner/repo.git`), and SSH scheme
/// (`ssh://git@gitlab.com/owner/repo.git`). Returns `Some` when the host
/// looks like GitLab — either its name contains "gitlab", or it has been
/// configured as a GitLab host in `glab`'s config file (self-hosted
/// instances such as `git.example.com`).
pub fn parse_gitlab_remote_url(remote_url: &str) -> Option<ForgeRepository> {
    let trimmed = trim_url_suffix(remote_url.trim());
    if trimmed.is_empty() {
        return None;
    }

    if let Some((host, path)) = parse_scp_like_remote(trimmed) {
        let resolved = resolve_ssh_hostname(host);
        if !is_gitlab_host(&resolved) {
            return None;
        }
        return gitlab_repository_from_path(&resolved, path);
    }

    let without_scheme = strip_scheme(trimmed).unwrap_or(trimmed);
    let without_user = without_scheme
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(without_scheme);
    let (host, path) = without_user.split_once('/')?;
    if !is_gitlab_host(host) {
        return None;
    }
    gitlab_repository_from_path(host, path)
}

/// Returns `true` when `host` looks like a GitLab instance: either its name
/// contains "gitlab" (covers `gitlab.com` and most public deployments) or
/// matches the default host `glab` is configured against (covers self-hosted
/// instances with custom domains like `git.example.com`).
fn is_gitlab_host(host: &str) -> bool {
    if host.contains("gitlab") {
        return true;
    }

    glab_default_host().is_some_and(|known| known.eq_ignore_ascii_case(host))
}

/// Read the default host configured in `glab` via `glab config get host`.
/// Used to recognize self-hosted GitLab instances whose domain doesn't
/// contain the substring "gitlab". Returns `None` when `glab` is missing,
/// the command fails, or no host is configured.
fn glab_default_host() -> Option<String> {
    let output = run_command_output("glab", None, ["config", "get", "host"]).ok()?;
    let host = output.trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
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

fn parse_gitlab_url_target(target: &str) -> Option<PullRequestTarget> {
    let without_scheme = strip_scheme(target)?;
    let trimmed = trim_url_suffix(without_scheme);
    let parts: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty()).collect();
    // Expected: [host, ...namespace..., "-", "merge_requests", "<iid>"]
    // Minimum: host + owner + repo + "-" + "merge_requests" + number = 6 parts
    if parts.len() < 6 {
        return None;
    }
    let host = parts[0];
    // Find the "-" separator that precedes "merge_requests"
    let dash_pos = parts[1..].iter().position(|&p| p == "-")? + 1;
    if parts.get(dash_pos + 1)? != &"merge_requests" {
        return None;
    }
    let number = parts.get(dash_pos + 2)?.parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }
    // Everything between host and "-" is the project path (may include subgroups)
    let project_parts = &parts[1..dash_pos];
    if project_parts.len() < 2 {
        return None;
    }
    let (owner_parts, repo_slice) = project_parts.split_at(project_parts.len() - 1);
    let owner = owner_parts.join("/");
    let repo = strip_git_suffix(repo_slice[0]);
    Some(PullRequestTarget::with_repository(
        ForgeRepository::gitlab(host, owner, repo),
        number,
        target,
    ))
}

fn parse_gitlab_repo_hash_target(target: &str) -> Option<PullRequestTarget> {
    let (repo_part, number_part) = target.split_once('#')?;
    let number = number_part.parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }
    let parts = repo_part
        .split('/')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>();
    // Need at least owner + repo (2 parts); host is optional prefix when no "." in first part
    let repository = if parts.len() >= 3 && parts[0].contains('.') {
        // [host, ...namespace..., repo]
        let host = parts[0];
        let (owner_parts, repo_slice) = parts[1..].split_at(parts.len() - 2);
        let owner = owner_parts.join("/");
        ForgeRepository::gitlab(host, owner, strip_git_suffix(repo_slice[0]))
    } else if parts.len() >= 2 {
        // [...namespace..., repo] — no host prefix
        let (owner_parts, repo_slice) = parts.split_at(parts.len() - 1);
        let owner = owner_parts.join("/");
        ForgeRepository::gitlab(DEFAULT_GITLAB_HOST, owner, strip_git_suffix(repo_slice[0]))
    } else {
        return None;
    };
    Some(PullRequestTarget::with_repository(
        repository, number, target,
    ))
}

fn gitlab_repository_from_path(host: &str, path: &str) -> Option<ForgeRepository> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return None;
    }
    let (owner_parts, repo_slice) = parts.split_at(parts.len() - 1);
    let owner = owner_parts.join("/");
    Some(ForgeRepository::gitlab(
        host,
        owner,
        strip_git_suffix(trim_url_suffix(repo_slice[0])),
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

/// Convert a bare unified diff (as output by `glab mr diff`) into git-style
/// by injecting `diff --git a/X b/X` headers before each `--- ` / `+++ ` pair.
///
/// `glab mr diff` emits plain unified diffs without git file headers, but the
/// tuicr parser requires `diff --git ` to detect file boundaries.
fn inject_git_diff_headers(diff: &str) -> String {
    let mut result = String::with_capacity(diff.len() + diff.lines().count() * 64);
    let mut lines = diff.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(old_raw) = line.strip_prefix("--- ") {
            // Peek at the next line to get the new path for added files.
            let new_raw = lines
                .peek()
                .and_then(|l| l.strip_prefix("+++ "))
                .unwrap_or(old_raw);
            // Determine the canonical path: prefer the non-/dev/null side.
            let path = if old_raw == "/dev/null" {
                new_raw
            } else {
                old_raw
            };
            // Strip a/ or b/ prefix if already present (shouldn't be for glab,
            // but be defensive).
            let path = path.trim_start_matches("a/").trim_start_matches("b/");
            result.push_str(&format!("diff --git a/{path} b/{path}\n"));
        }
        result.push_str(line);
        result.push('\n');
    }
    result
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
    let resolved = read_ssh_hostname(alias);
    normalize_ssh_transport_host(&resolved).to_string()
}

fn read_ssh_hostname(alias: &str) -> String {
    let Ok(home) = std::env::var("HOME") else {
        return alias.to_string();
    };
    let path = PathBuf::from(home).join(".ssh/config");
    let Ok(content) = fs::read_to_string(path) else {
        return alias.to_string();
    };
    resolve_ssh_hostname_from_config(alias, &content)
}

/// Map GitLab's SSH-over-443 transport host back to its API host. Users behind
/// networks that block port 22 set `Hostname altssh.gitlab.com` for
/// `gitlab.com` in `~/.ssh/config`; that's correct for transport but
/// `altssh.gitlab.com` is not the API host.
fn normalize_ssh_transport_host(host: &str) -> &str {
    if host.eq_ignore_ascii_case("altssh.gitlab.com") {
        "gitlab.com"
    } else {
        host
    }
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

fn map_glab_error(error: GlabCommandError, host: &str) -> TuicrError {
    match error {
        GlabCommandError::MissingGlab => TuicrError::Forge(
            "GitLab integration requires `glab`.\nInstall GitLab CLI and run `glab auth login`."
                .to_string(),
        ),
        GlabCommandError::Failed { stderr, .. } if looks_like_auth_failure(&stderr) => {
            TuicrError::Forge(format!(
                "GitLab authentication failed.\nRun `glab auth login` for {host}."
            ))
        }
        GlabCommandError::Failed { stderr, status } => {
            let detail = if stderr.is_empty() {
                status
                    .map(|code| format!("glab exited with status {code}"))
                    .unwrap_or_else(|| "glab command failed".to_string())
            } else {
                stderr
            };
            TuicrError::Forge(format!("GitLab command failed: {detail}"))
        }
    }
}

/// Compute the GitLab `line_code` for a diff note.
///
/// Format: `{SHA1(file_path)}_{old_line}_{new_line}`
/// For a new-side (right) comment: old_line = 0.
/// For an old-side (left) comment: new_line = 0.
fn gl_line_code(file_path: &str, old_line: u32, new_line: u32) -> String {
    let hash = format!("{:x}", Sha1::digest(file_path.as_bytes()));
    format!("{hash}_{old_line}_{new_line}")
}

/// Build one endpoint of a GitLab `line_range` position entry.
///
/// GitLab expects each endpoint to carry the `type` ("new" / "old"), the
/// integer line number on that side, and the `line_code` so the server can
/// anchor the range without re-walking the diff.
fn gl_range_endpoint(new_path: &str, side: GhSide, line: u32) -> serde_json::Value {
    match side {
        GhSide::Right => serde_json::json!({
            "type": "new",
            "new_line": line,
            "line_code": gl_line_code(new_path, 0, line),
        }),
        GhSide::Left => serde_json::json!({
            "type": "old",
            "old_line": line,
            "line_code": gl_line_code(new_path, line, 0),
        }),
    }
}

/// Inspect a `glab api graphql` response for logical errors.
///
/// GraphQL returns HTTP 200 even when a mutation fails, so a successful exit
/// status is not enough. The response body must be parsed. Errors surface
/// either at the top level (`errors`) or inside the mutation payload
/// (`data.<mutation>.errors`). A non-empty array in either place is a failure.
fn check_graphql_errors(output: &str, mutation: &str) -> Result<()> {
    let value: serde_json::Value = match serde_json::from_str(output) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };

    // Payload errors (`data.<mutation>.errors`) are `[String!]`. Top-level
    // GraphQL errors are objects carrying a `message` field. Prefer that
    // message; fall back to the raw value so nothing is silently dropped.
    let collect = |array: Option<&serde_json::Value>| -> Vec<String> {
        array
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        if let Some(s) = item.as_str() {
                            s.to_string()
                        } else if let Some(message) = item.get("message").and_then(|m| m.as_str()) {
                            message.to_string()
                        } else {
                            item.to_string()
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let mut messages = collect(value.get("errors"));
    messages.extend(collect(value.pointer(&format!("/data/{mutation}/errors"))));

    if messages.is_empty() {
        Ok(())
    } else {
        Err(TuicrError::Forge(format!(
            "GitLab request-changes failed: {}",
            messages.join(", ")
        )))
    }
}

fn map_create_notes_error(error: GlabCommandError, host: &str) -> TuicrError {
    if let GlabCommandError::Failed { ref stderr, .. } = error
        && looks_like_permission_failure(stderr)
    {
        return TuicrError::Forge(
            "Cannot submit review: GitLab token lacks merge request write permission.".to_string(),
        );
    }
    map_glab_error(error, host)
}

fn looks_like_auth_failure(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("glab auth login")
        || lower.contains("not logged in")
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
}

fn malformed_target<T>(input: &str) -> Result<T> {
    Err(TuicrError::Forge(format!(
        "Malformed GitLab merge request target: `{input}`"
    )))
}

/// Append `msg` to `/tmp/tuicr-glab-debug.log` when `TUICR_GLAB_DEBUG` is set.
/// Uses a file so the TUI doesn't swallow the output.
fn glab_debug_log(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/tuicr-glab-debug.log")
    {
        let _ = f.write_all(msg.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::forge::submit::{GhSide, InlineComment};
    use crate::forge::traits::{
        CreateReviewRequest, ForgeRepository, PullRequestDetails, PullRequestListQuery,
        PullRequestListScope,
    };

    /// Mock runner that records (args, stdin) calls.
    struct RecordingRunner {
        calls: RefCell<Vec<(Vec<String>, Option<String>)>>,
        responses: RefCell<Vec<String>>,
    }

    impl RecordingRunner {
        fn new_with_responses(responses: Vec<String>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                responses: RefCell::new(responses),
            }
        }
    }

    impl GlabCommandRunner for RecordingRunner {
        fn run(&self, args: &[String]) -> GlabCommandResult<String> {
            self.calls.borrow_mut().push((args.to_vec(), None));
            let resp = self
                .responses
                .borrow_mut()
                .drain(..1)
                .next()
                .unwrap_or_default();
            Ok(resp)
        }

        fn run_with_stdin(&self, args: &[String], stdin: &str) -> GlabCommandResult<String> {
            self.calls
                .borrow_mut()
                .push((args.to_vec(), Some(stdin.to_string())));
            let resp = self
                .responses
                .borrow_mut()
                .drain(..1)
                .next()
                .unwrap_or_default();
            Ok(resp)
        }
    }

    #[test]
    fn list_pull_requests_can_filter_to_review_requested() {
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let runner = RecordingRunner::new_with_responses(vec!["[]".to_string()]);
        let backend = GitLabGlabBackend::with_runner(None, runner);

        backend
            .list_pull_requests(PullRequestListQuery::first_page_with_scope(
                repo,
                1,
                PullRequestListScope::ReviewRequested,
            ))
            .unwrap();

        let calls = backend.runner.calls.borrow();
        assert_eq!(
            calls[0].0,
            vec![
                "mr",
                "list",
                "--repo",
                "owner/repo",
                "--output",
                "json",
                "--per-page",
                "2",
                "--reviewer=@me",
            ]
        );
    }

    fn make_pr_details(repo: ForgeRepository) -> PullRequestDetails {
        PullRequestDetails {
            repository: repo,
            number: 42,
            title: "Test MR".to_string(),
            url: "https://gitlab.com/owner/repo/-/merge_requests/42".to_string(),
            state: "OPEN".to_string(),
            is_draft: false,
            author: None,
            head_ref_name: "feature".to_string(),
            base_ref_name: "main".to_string(),
            head_sha: "headsha1".to_string(),
            base_sha: "basesha1".to_string(),
            body: String::new(),
            updated_at: None,
            closed: false,
            merged_at: None,
            diff_start_sha: Some("startsha1".to_string()),
        }
    }

    #[test]
    fn should_send_inline_comment_as_json_with_integer_line_number() {
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/lib.rs".into(),
            line: 15,
            side: GhSide::Right,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            old_path: None,
            body: "nice work".to_string(),
            comment_id: "c1".to_string(),
        };
        let response = r#"{"id":"disc-abc","individual_note":false}"#.to_string();
        let runner = RecordingRunner::new_with_responses(vec![response]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);
        let request = CreateReviewRequest {
            event: crate::forge::submit::SubmitEvent::Comment,
            commit_id: "headsha1",
            body: "",
            comments: &[inline],
        };
        backend.create_review(&pr, request).unwrap();
        let calls = backend.runner.calls.borrow();
        // Should be one call (the inline comment)
        assert_eq!(calls.len(), 1);
        let (args, stdin) = &calls[0];
        // Must use --input - for JSON body
        assert!(
            args.contains(&"--input".to_string()),
            "expected --input flag"
        );
        assert!(args.contains(&"-".to_string()), "expected - stdin flag");
        assert!(
            args.contains(&"Content-Type: application/json".to_string()),
            "expected JSON Content-Type"
        );
        let body: serde_json::Value = serde_json::from_str(stdin.as_ref().unwrap()).unwrap();
        let position = &body["position"];
        // Line number must be an integer, not a string
        assert_eq!(
            position["new_line"],
            serde_json::Value::Number(15.into()),
            "new_line must be integer 15"
        );
        assert_eq!(
            position["old_line"],
            serde_json::Value::Null,
            "old_line absent or null for Right-side comment"
        );
        assert_eq!(position["position_type"], "text");
        assert_eq!(position["base_sha"], "basesha1");
        assert_eq!(position["start_sha"], "startsha1");
        assert_eq!(position["head_sha"], "headsha1");
        assert_eq!(position["new_path"], "src/lib.rs");
        assert_eq!(position["old_path"], "src/lib.rs");
        assert_eq!(body["body"], "nice work");
    }

    #[test]
    fn should_send_left_side_inline_comment_with_old_line() {
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/main.rs".into(),
            line: 7,
            side: GhSide::Left,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            old_path: None,
            body: "old code".to_string(),
            comment_id: "c2".to_string(),
        };
        let response = r#"{"id":"disc-def","individual_note":false}"#.to_string();
        let runner = RecordingRunner::new_with_responses(vec![response]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);
        let request = CreateReviewRequest {
            event: crate::forge::submit::SubmitEvent::Comment,
            commit_id: "headsha1",
            body: "",
            comments: &[inline],
        };
        backend.create_review(&pr, request).unwrap();
        let calls = backend.runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        let (_, stdin) = &calls[0];
        let body: serde_json::Value = serde_json::from_str(stdin.as_ref().unwrap()).unwrap();
        let position = &body["position"];
        assert_eq!(
            position["old_line"],
            serde_json::Value::Number(7.into()),
            "old_line must be integer 7"
        );
        // new_line should be absent (not set)
        assert!(
            position.get("new_line").is_none() || position["new_line"] == serde_json::Value::Null,
            "new_line should be absent for Left-side comment"
        );
    }

    #[test]
    fn should_send_line_range_for_multi_line_range_comment() {
        // Visual/range comments must carry GitLab's `line_range` so the
        // discussion anchors to the full selection instead of collapsing to
        // the end line.
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/lib.rs".into(),
            line: 30,
            side: GhSide::Right,
            counterpart_line: None,
            start_line: Some(25),
            start_side: Some(GhSide::Right),
            old_path: None,
            body: "range comment".to_string(),
            comment_id: "c-range".to_string(),
        };
        let response = r#"{"id":"disc-range","individual_note":false}"#.to_string();
        let runner = RecordingRunner::new_with_responses(vec![response]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);
        let request = CreateReviewRequest {
            event: crate::forge::submit::SubmitEvent::Comment,
            commit_id: "headsha1",
            body: "",
            comments: &[inline],
        };
        backend.create_review(&pr, request).unwrap();
        let calls = backend.runner.calls.borrow();
        let (_, stdin) = &calls[0];
        let body: serde_json::Value = serde_json::from_str(stdin.as_ref().unwrap()).unwrap();
        let position = &body["position"];
        assert_eq!(position["line_range"]["start"]["type"], "new");
        assert_eq!(
            position["line_range"]["start"]["new_line"],
            serde_json::Value::Number(25.into())
        );
        assert_eq!(position["line_range"]["end"]["type"], "new");
        assert_eq!(
            position["line_range"]["end"]["new_line"],
            serde_json::Value::Number(30.into())
        );
        // line_code is required by GitLab to anchor the endpoint.
        assert!(position["line_range"]["start"]["line_code"].is_string());
        assert!(position["line_range"]["end"]["line_code"].is_string());
    }

    #[test]
    fn should_send_distinct_old_and_new_path_for_renamed_file() {
        // Renamed files carry different old_path and new_path. GitLab needs
        // both to anchor a position; collapsing them to the same string
        // breaks comments on renamed files.
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/new_name.rs".into(),
            line: 5,
            side: GhSide::Right,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            old_path: Some("src/old_name.rs".into()),
            body: "renamed file comment".to_string(),
            comment_id: "c-rename".to_string(),
        };
        let response = r#"{"id":"disc-rename","individual_note":false}"#.to_string();
        let runner = RecordingRunner::new_with_responses(vec![response]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);
        let request = CreateReviewRequest {
            event: crate::forge::submit::SubmitEvent::Comment,
            commit_id: "headsha1",
            body: "",
            comments: &[inline],
        };
        backend.create_review(&pr, request).unwrap();
        let calls = backend.runner.calls.borrow();
        let (_, stdin) = &calls[0];
        let body: serde_json::Value = serde_json::from_str(stdin.as_ref().unwrap()).unwrap();
        let position = &body["position"];
        assert_eq!(position["old_path"], "src/old_name.rs");
        assert_eq!(position["new_path"], "src/new_name.rs");
    }

    #[test]
    fn should_send_both_line_numbers_for_context_line() {
        // Context lines (unchanged) have both old and new line numbers.
        // GitLab requires both in the position object to compute a valid
        // line_code; sending only one causes a 400 "line_code can't be blank".
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/lib.rs".into(),
            line: 20,
            side: GhSide::Right,
            counterpart_line: Some(18), // old_lineno for this context line
            start_line: None,
            start_side: None,
            old_path: None,
            body: "context comment".to_string(),
            comment_id: "c3".to_string(),
        };
        let response = r#"{"id":"disc-ctx","individual_note":false}"#.to_string();
        let runner = RecordingRunner::new_with_responses(vec![response]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);
        let request = CreateReviewRequest {
            event: crate::forge::submit::SubmitEvent::Comment,
            commit_id: "headsha1",
            body: "",
            comments: &[inline],
        };
        backend.create_review(&pr, request).unwrap();
        let calls = backend.runner.calls.borrow();
        let (_, stdin) = &calls[0];
        let body: serde_json::Value = serde_json::from_str(stdin.as_ref().unwrap()).unwrap();
        let position = &body["position"];
        assert_eq!(
            position["new_line"],
            serde_json::Value::Number(20.into()),
            "new_line must be the primary line"
        );
        assert_eq!(
            position["old_line"],
            serde_json::Value::Number(18.into()),
            "old_line must be the counterpart for context lines"
        );
    }

    #[test]
    fn should_call_approve_endpoint_for_approve() {
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/lib.rs".into(),
            line: 15,
            side: GhSide::Right,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            old_path: None,
            body: "looks good".to_string(),
            comment_id: "c-approve".to_string(),
        };
        let runner = RecordingRunner::new_with_responses(vec![
            r#"{"id":"disc-approve","individual_note":false}"#.to_string(),
            String::new(),
        ]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);
        let request = CreateReviewRequest {
            event: crate::forge::submit::SubmitEvent::Approve,
            commit_id: "headsha1",
            body: "",
            comments: &[inline],
        };
        let response = backend.create_review(&pr, request).unwrap();
        assert_eq!(response.state, "COMMENTED");

        let calls = backend.runner.calls.borrow();
        // Discussion POST, then the approve call.
        assert_eq!(calls.len(), 2);
        let approve_args = &calls[1].0;
        assert!(
            approve_args
                .iter()
                .any(|a| a == "projects/owner%2Frepo/merge_requests/42/approve"),
            "expected approve endpoint, got {approve_args:?}"
        );
    }

    #[test]
    fn should_post_comments_then_request_changes_mutation() {
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/lib.rs".into(),
            line: 15,
            side: GhSide::Right,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            old_path: None,
            body: "please change this".to_string(),
            comment_id: "c-rc".to_string(),
        };
        let runner = RecordingRunner::new_with_responses(vec![
            // body note POST
            r#"{"id":1}"#.to_string(),
            // discussion POST
            r#"{"id":"disc-rc","individual_note":false}"#.to_string(),
            // graphql mutation
            r#"{"data":{"mergeRequestRequestChanges":{"errors":[]}}}"#.to_string(),
        ]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);
        let request = CreateReviewRequest {
            event: crate::forge::submit::SubmitEvent::RequestChanges,
            commit_id: "headsha1",
            body: "overall please address the comments",
            comments: &[inline],
        };
        let response = backend.create_review(&pr, request).unwrap();
        assert_eq!(response.state, "CHANGES_REQUESTED");

        let calls = backend.runner.calls.borrow();
        // body note POST, discussion POST, graphql mutation
        assert_eq!(calls.len(), 3);

        // Body note posted to the notes endpoint.
        assert!(
            calls[0]
                .0
                .iter()
                .any(|a| a == "projects/owner%2Frepo/merge_requests/42/notes"),
            "expected body note endpoint, got {:?}",
            calls[0].0
        );
        // Inline comment posted as a discussion.
        assert!(
            calls[1]
                .0
                .iter()
                .any(|a| a == "projects/owner%2Frepo/merge_requests/42/discussions"),
            "expected discussion endpoint, got {:?}",
            calls[1].0
        );

        // Final call is the graphql request-changes mutation.
        let graphql_args = &calls[2].0;
        assert_eq!(graphql_args[0], "api");
        assert_eq!(graphql_args[1], "graphql");
        assert!(
            graphql_args
                .iter()
                .any(|a| a.contains("mergeRequestRequestChanges")),
            "expected mergeRequestRequestChanges in query, got {graphql_args:?}"
        );
        // projectPath must be the un-encoded full path.
        assert!(
            graphql_args.iter().any(|a| a == "projectPath=owner/repo"),
            "expected un-encoded projectPath, got {graphql_args:?}"
        );
        // iid is the MR number, passed as a raw string field.
        assert!(
            graphql_args.iter().any(|a| a == "iid=42"),
            "expected iid=42, got {graphql_args:?}"
        );
    }

    #[test]
    fn should_error_when_request_changes_mutation_returns_errors() {
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/lib.rs".into(),
            line: 15,
            side: GhSide::Right,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            old_path: None,
            body: "please change this".to_string(),
            comment_id: "c-rc-err".to_string(),
        };
        let runner = RecordingRunner::new_with_responses(vec![
            r#"{"id":"disc-rc","individual_note":false}"#.to_string(),
            r#"{"data":{"mergeRequestRequestChanges":{"errors":["You are not a reviewer"]}}}"#
                .to_string(),
        ]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);
        let request = CreateReviewRequest {
            event: crate::forge::submit::SubmitEvent::RequestChanges,
            commit_id: "headsha1",
            body: "",
            comments: &[inline],
        };
        let err = backend.create_review(&pr, request).unwrap_err();
        match err {
            TuicrError::Forge(msg) => {
                assert!(
                    msg.contains("You are not a reviewer"),
                    "expected reviewer error message, got {msg}"
                );
            }
            other => panic!("expected TuicrError::Forge, got {other:?}"),
        }
    }

    #[test]
    fn check_graphql_errors_extracts_message_from_top_level_error_objects() {
        // Top-level GraphQL errors are objects with a `message`, not strings.
        let output = r#"{"errors":[{"message":"Authentication required"}]}"#;
        let err = check_graphql_errors(output, "mergeRequestRequestChanges").unwrap_err();
        match err {
            TuicrError::Forge(msg) => assert!(
                msg.contains("Authentication required") && !msg.contains('{'),
                "expected clean message, got {msg}"
            ),
            other => panic!("expected TuicrError::Forge, got {other:?}"),
        }
    }

    #[test]
    fn check_graphql_errors_ok_on_empty_errors_and_non_json() {
        // Empty payload-error array is success.
        check_graphql_errors(r#"{"data":{"m":{"errors":[]}}}"#, "m").unwrap();
        // Non-JSON output is treated as success; run_glab already fails on a
        // non-zero exit before this helper runs.
        check_graphql_errors("not json", "m").unwrap();
    }

    #[test]
    fn should_post_draft_notes_for_draft_submission() {
        // `:submit draft` on GitLab must create draft notes (the pending-review
        // primitive) for both the body and inline comments, mirroring GitHub's
        // pending review. No discussion, approve, or GraphQL call should fire.
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/lib.rs".into(),
            line: 15,
            side: GhSide::Right,
            counterpart_line: Some(12),
            start_line: None,
            start_side: None,
            old_path: None,
            body: "inline draft".to_string(),
            comment_id: "c1".to_string(),
        };
        let runner = RecordingRunner::new_with_responses(vec![
            r#"{"id":10}"#.to_string(),
            r#"{"id":22}"#.to_string(),
        ]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);
        let request = CreateReviewRequest {
            event: crate::forge::submit::SubmitEvent::Draft,
            commit_id: "headsha1",
            body: "overall draft",
            comments: &[inline],
        };
        let response = backend.create_review(&pr, request).unwrap();
        assert_eq!(response.state, "PENDING");
        // draft_notes return a numeric `id`. The id is captured from the first
        // inline note (matching the discussion path), so it must be parsed from
        // the inline response (22), not left at 0.
        assert_eq!(response.id, 22, "inline draft note id should be reported");

        let calls = backend.runner.calls.borrow();
        assert_eq!(calls.len(), 2, "expected one body note and one inline note");

        // Every call must target a draft_notes endpoint; none may publish or
        // fall back to discussions/approve/graphql.
        for (args, _) in calls.iter() {
            let endpoint = &args[1];
            assert!(
                endpoint.ends_with("/draft_notes"),
                "expected draft_notes endpoint, got {endpoint}"
            );
            assert!(
                !endpoint.contains("discussions"),
                "draft path must not hit discussions"
            );
            assert!(
                !args.iter().any(|a| a.contains("approve") || a == "graphql"),
                "draft path must not approve or run graphql"
            );
        }

        // Body note: { "note": <body> }, no position.
        let (_, body_stdin) = &calls[0];
        let body: serde_json::Value = serde_json::from_str(body_stdin.as_ref().unwrap()).unwrap();
        assert_eq!(body["note"], "overall draft");
        assert!(
            body.get("body").is_none(),
            "draft must use `note`, not `body`"
        );
        assert!(body.get("position").is_none(), "body note has no position");

        // Inline note: `note` + position carrying the expected line numbers.
        let (_, inline_stdin) = &calls[1];
        let inline: serde_json::Value =
            serde_json::from_str(inline_stdin.as_ref().unwrap()).unwrap();
        assert_eq!(inline["note"], "inline draft");
        assert!(
            inline.get("body").is_none(),
            "inline draft must use `note`, not `body`"
        );
        let position = &inline["position"];
        assert_eq!(position["new_line"], serde_json::Value::Number(15.into()));
        assert_eq!(position["old_line"], serde_json::Value::Number(12.into()));
    }

    #[test]
    fn should_not_publish_draft_notes() {
        // Draft submissions stop at creating draft notes. Publishing is left to
        // the GitLab web UI, so no call may touch bulk_publish or /publish.
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/lib.rs".into(),
            line: 15,
            side: GhSide::Right,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            old_path: None,
            body: "inline draft".to_string(),
            comment_id: "c1".to_string(),
        };
        let runner = RecordingRunner::new_with_responses(vec![
            r#"{"id":1}"#.to_string(),
            r#"{"id":2}"#.to_string(),
        ]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);
        let request = CreateReviewRequest {
            event: crate::forge::submit::SubmitEvent::Draft,
            commit_id: "headsha1",
            body: "overall draft",
            comments: &[inline],
        };
        backend.create_review(&pr, request).unwrap();

        let calls = backend.runner.calls.borrow();
        for (args, _) in calls.iter() {
            assert!(
                !args
                    .iter()
                    .any(|a| a.contains("bulk_publish") || a.contains("/publish")),
                "draft path must not publish"
            );
        }
    }

    #[test]
    fn should_post_comment_submission_to_notes_and_discussions() {
        // Guard against the draft branching regressing the default Comment path:
        // body goes to `/notes`, inline comments to `/discussions`.
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/lib.rs".into(),
            line: 15,
            side: GhSide::Right,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            old_path: None,
            body: "inline comment".to_string(),
            comment_id: "c1".to_string(),
        };
        let runner = RecordingRunner::new_with_responses(vec![
            String::new(),
            r#"{"id":"disc-abc"}"#.to_string(),
        ]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);
        let request = CreateReviewRequest {
            event: crate::forge::submit::SubmitEvent::Comment,
            commit_id: "headsha1",
            body: "overall comment",
            comments: &[inline],
        };
        let response = backend.create_review(&pr, request).unwrap();
        assert_eq!(response.state, "COMMENTED");

        let calls = backend.runner.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].0[1].ends_with("/notes"));
        assert!(calls[1].0[1].ends_with("/discussions"));
        for (args, _) in calls.iter() {
            assert!(
                !args.iter().any(|a| a.contains("draft_notes")),
                "comment path must not hit draft_notes"
            );
        }
    }

    #[test]
    fn should_parse_gitlab_https_remote_url() {
        let repo = parse_gitlab_remote_url("https://gitlab.com/owner/repo.git").unwrap();
        assert_eq!(repo, ForgeRepository::gitlab("gitlab.com", "owner", "repo"));
    }

    #[test]
    fn should_parse_gitlab_ssh_remote_url() {
        let repo = parse_gitlab_remote_url("git@gitlab.com:owner/repo.git").unwrap();
        assert_eq!(repo, ForgeRepository::gitlab("gitlab.com", "owner", "repo"));
    }

    #[test]
    fn should_parse_gitlab_self_hosted_https() {
        let repo = parse_gitlab_remote_url("https://gitlab.example.com/owner/repo.git").unwrap();
        assert_eq!(
            repo,
            ForgeRepository::gitlab("gitlab.example.com", "owner", "repo")
        );
    }

    #[test]
    fn should_ignore_github_remote_url() {
        assert!(parse_gitlab_remote_url("https://github.com/owner/repo.git").is_none());
        assert!(parse_gitlab_remote_url("git@github.com:owner/repo.git").is_none());
    }

    #[test]
    fn repo_arg_uses_full_url_for_non_default_host() {
        let repo = ForgeRepository::gitlab("git.example.com", "group/team", "project");
        assert_eq!(
            GitLabGlabBackend::<SystemGlabRunner>::repo_arg(&repo),
            "https://git.example.com/group/team/project"
        );
    }

    #[test]
    fn repo_arg_uses_slug_for_default_host() {
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        assert_eq!(
            GitLabGlabBackend::<SystemGlabRunner>::repo_arg(&repo),
            "owner/repo"
        );
    }

    #[test]
    fn should_parse_gitlab_nested_group_remote_url() {
        let repo = parse_gitlab_remote_url("git@gitlab.com:technosylva/ai/synapse.git").unwrap();
        assert_eq!(repo.owner, "technosylva/ai");
        assert_eq!(repo.name, "synapse");
        assert_eq!(repo.slug(), "technosylva/ai/synapse");
    }

    #[test]
    fn should_parse_gitlab_mr_url_target() {
        let target =
            parse_pull_request_target_gitlab("https://gitlab.com/owner/repo/-/merge_requests/42")
                .unwrap();
        assert_eq!(target.number, 42);
        let repo = target.repository.unwrap();
        assert_eq!(repo.host, "gitlab.com");
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
        assert_eq!(repo.kind, crate::forge::traits::ForgeKind::GitLab);
    }

    #[test]
    fn should_parse_gitlab_nested_group_url_target() {
        let target = parse_pull_request_target_gitlab(
            "https://gitlab.com/technosylva/ai/synapse/-/merge_requests/33",
        )
        .unwrap();
        assert_eq!(target.number, 33);
        let repo = target.repository.unwrap();
        assert_eq!(repo.host, "gitlab.com");
        assert_eq!(repo.owner, "technosylva/ai");
        assert_eq!(repo.name, "synapse");
        assert_eq!(repo.slug(), "technosylva/ai/synapse");
    }

    #[test]
    fn should_parse_gitlab_repo_hash_target() {
        let target = parse_pull_request_target_gitlab("owner/repo#42").unwrap();
        assert_eq!(target.number, 42);
        let repo = target.repository.unwrap();
        assert_eq!(repo.host, DEFAULT_GITLAB_HOST);
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
    }

    #[test]
    fn should_parse_gitlab_numeric_target() {
        let target = parse_pull_request_target_gitlab("123").unwrap();
        assert_eq!(target.number, 123);
        assert!(target.repository.is_none());
    }

    #[test]
    fn normalizes_gitlab_ssh_over_443_transport_host() {
        assert_eq!(
            normalize_ssh_transport_host("altssh.gitlab.com"),
            "gitlab.com"
        );
        assert_eq!(
            normalize_ssh_transport_host("AltSSH.GitLab.com"),
            "gitlab.com"
        );
        assert_eq!(normalize_ssh_transport_host("gitlab.com"), "gitlab.com");
        assert_eq!(
            normalize_ssh_transport_host("gitlab.example.com"),
            "gitlab.example.com"
        );
    }

    #[test]
    fn should_reject_empty_target() {
        assert!(parse_pull_request_target_gitlab("").is_err());
        assert!(parse_pull_request_target_gitlab("  ").is_err());
    }
}
