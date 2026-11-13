use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use sha1::{Digest, Sha1};

use crate::error::{Result, TuicrError};
use crate::forge::local_git::read_blob;
use crate::forge::remote_comments::{RemoteReviewSummary, RemoteReviewThread};
use crate::forge::traits::{
    ForgeBackend, ForgeFileLinesRequest, ForgeRepository, GhCreateReviewResponse,
    PagedPullRequests, PullRequestCommit, PullRequestDetails, PullRequestListQuery,
    PullRequestListScope, PullRequestReviewMetadata, PullRequestReviewRecord, PullRequestTarget,
};
use crate::forge::{forge_path, percent_encode_component};
use crate::model::{DiffLine, FilePatch, LineOrigin};
use crate::process::{
    CommandOutputError, CommandOutputErrorKind, run_command_output, run_command_output_with_stdin,
};
use crate::vcs::git::raw::run_git_diff;
use crate::vcs::slice_context_lines;

use super::instance::{self, SubfolderCache};
use super::models::{
    GlabApprovalState, GlabCommit, GlabDiff, GlabDiscussion, GlabDraftNote, GlabMrDetails,
    GlabMrSummary, GlabMrVersion, GlabUser, draft_notes_to_review,
};
use crate::forge::submit::{DiffAnchor, GhSide, SubmitEvent};
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

/// Percent-encode a project path as `owner%2Frepo` for GitLab project API
/// paths. Build the path itself with [`GitLabGlabBackend::project_path`] —
/// `owner` can carry an install subfolder that must not reach the API.
fn gl_encode_project_path(project_path: &str) -> String {
    project_path.replace('/', "%2F")
}

/// Percent-encode a file path for use in GitLab repository file API endpoints.
fn gl_encode_file_path(path: &str) -> String {
    percent_encode_component(path)
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn version_head_at(versions: &[GlabMrVersion], submitted_at: &DateTime<Utc>) -> Option<String> {
    versions
        .iter()
        .filter_map(|version| {
            let created_at = version.created_at.as_ref()?;
            if created_at <= submitted_at && !version.head_commit_sha.is_empty() {
                Some((created_at, version.head_commit_sha.as_str()))
            } else {
                None
            }
        })
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, sha)| sha.to_string())
}

#[derive(Debug, Clone)]
pub struct GitLabGlabBackend<R = SystemGlabRunner> {
    default_repository: Option<ForgeRepository>,
    runner: R,
    local_checkout: Option<PathBuf>,
    subfolders: SubfolderCache,
}

impl GitLabGlabBackend<SystemGlabRunner> {
    pub fn new(default_repository: Option<ForgeRepository>) -> Self {
        Self {
            default_repository,
            runner: SystemGlabRunner,
            local_checkout: None,
            subfolders: SubfolderCache::default(),
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
            subfolders: SubfolderCache::default(),
        }
    }

    /// Like [`Self::with_runner`], with `host`'s install subfolder already
    /// known. Keeps the endpoint tests off both `GITLAB_SUBFOLDER` (process
    /// state other tests mutate) and the runner's response queue.
    #[cfg(test)]
    fn with_runner_and_subfolder(
        default_repository: Option<ForgeRepository>,
        runner: R,
        host: &str,
        subfolder: Option<&str>,
    ) -> Self {
        Self {
            default_repository,
            runner,
            local_checkout: None,
            subfolders: SubfolderCache::seeded(host, subfolder),
        }
    }

    /// The path prefix `glab` prepends to this host's API URLs when GitLab is
    /// installed under a relative URL. `gitlab.com` can't be, and skipping the
    /// lookup there keeps the common case free of a process spawn.
    fn install_subfolder(&self, host: &str) -> Option<String> {
        if host.eq_ignore_ascii_case(DEFAULT_GITLAB_HOST) {
            return None;
        }
        self.subfolders.resolve(host, &self.runner)
    }

    /// The project path as the GitLab API names it: `owner/name` with the
    /// instance's install subfolder removed. The GitLab analogue of
    /// `gerrit_project()` / `azure_coords()`, and the only supported way from a
    /// [`ForgeRepository`] to an API project path.
    ///
    /// A relative-URL install's subfolder is indistinguishable from a top-level
    /// group in a clone URL, so it ends up inside `repo.owner` — deliberately,
    /// since `owner` is persisted in session slugs and `PrSessionKey` and is
    /// also parsed from host-less targets that could never look a subfolder up.
    /// `glab` prefixes that subfolder onto every API URL itself, so leaving it
    /// in the project id requests
    /// `/<sub>/api/v4/projects/<sub>%2Fgroup%2Fproject` and 404s.
    fn project_path(&self, repo: &ForgeRepository) -> String {
        let full_path = format!("{}/{}", repo.owner, repo.name);
        match self.install_subfolder(&repo.host) {
            Some(subfolder) => {
                instance::strip_subfolder_prefix(&full_path, &subfolder).unwrap_or(full_path)
            }
            None => full_path,
        }
    }

    /// [`Self::project_path`], percent-encoded for an endpoint segment.
    fn api_project(&self, repo: &ForgeRepository) -> String {
        gl_encode_project_path(&self.project_path(repo))
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
        self.runner.run(&args).map_err(|err| {
            // A 404 against a self-hosted host with no subfolder configured is
            // the one case tuicr cannot fix itself. The lookup is already
            // cached by the time a call fails, so this costs nothing.
            let suggest_subfolder = !host.eq_ignore_ascii_case(DEFAULT_GITLAB_HOST)
                && self.install_subfolder(host).is_none();
            map_glab_error(err, host, suggest_subfolder)
        })
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

    fn run_api(&self, repo: &ForgeRepository, endpoint: String) -> Result<String> {
        let mut args = vec!["api".to_string()];
        args.extend(Self::api_hostname_args(repo));
        args.push(endpoint);
        self.run_glab(args, &repo.host)
    }

    fn current_username(&self, repo: &ForgeRepository) -> Result<Option<String>> {
        let output = self.run_api(repo, "user".to_string())?;
        let user: GlabUser = serde_json::from_str(&output)?;
        Ok(non_empty(&user.username))
    }

    /// The viewer's unpublished draft notes on the MR. GitLab hides them from
    /// `/discussions`, so this is the only way to read `:submit draft` back.
    fn list_draft_notes(&self, pr: &PullRequestDetails) -> Result<Vec<GlabDraftNote>> {
        let project = self.api_project(&pr.repository);
        let mut notes = Vec::new();
        for page in 1..=100 {
            let endpoint = format!(
                "projects/{}/merge_requests/{}/draft_notes?per_page=100&page={}",
                project, pr.number, page,
            );
            let output = self.run_api(&pr.repository, endpoint)?;
            let rows: Vec<GlabDraftNote> = serde_json::from_str(&output)?;
            let received = rows.len();
            notes.extend(rows);
            if received < 100 {
                break;
            }
        }
        Ok(notes)
    }

    fn list_mr_versions(&self, pr: &PullRequestDetails) -> Result<Vec<GlabMrVersion>> {
        let project = self.api_project(&pr.repository);
        let mut versions = Vec::new();
        for page in 1..=100 {
            let endpoint = format!(
                "projects/{}/merge_requests/{}/versions?per_page=100&page={}",
                project, pr.number, page,
            );
            let output = self.run_api(&pr.repository, endpoint)?;
            let rows: Vec<GlabMrVersion> = serde_json::from_str(&output)?;
            let received = rows.len();
            versions.extend(rows);
            if received < 100 {
                break;
            }
        }
        Ok(versions)
    }

    fn list_mr_approval_records(
        &self,
        pr: &PullRequestDetails,
        versions: &[GlabMrVersion],
    ) -> Result<Vec<PullRequestReviewRecord>> {
        let project = self.api_project(&pr.repository);
        let endpoint = format!(
            "projects/{}/merge_requests/{}/approvals",
            project, pr.number
        );
        let output = self.run_api(&pr.repository, endpoint)?;
        let approvals: GlabApprovalState = serde_json::from_str(&output)?;
        Ok(approvals
            .approved_by
            .into_iter()
            .map(|approval| {
                let commit_oid = approval
                    .approved_at
                    .as_ref()
                    .and_then(|approved_at| version_head_at(versions, approved_at));
                PullRequestReviewRecord {
                    author: non_empty(&approval.user.username),
                    submitted_at: approval.approved_at,
                    commit_oid,
                }
            })
            .collect())
    }

    fn list_mr_discussion_review_records(
        &self,
        pr: &PullRequestDetails,
        versions: &[GlabMrVersion],
    ) -> Result<Vec<PullRequestReviewRecord>> {
        let project = self.api_project(&pr.repository);
        let mut records = Vec::new();
        for page in 1..=100 {
            let endpoint = format!(
                "projects/{}/merge_requests/{}/discussions?per_page=100&page={}",
                project, pr.number, page,
            );
            let output = self.run_api(&pr.repository, endpoint)?;
            let discussions: Vec<GlabDiscussion> = serde_json::from_str(&output)?;
            let received = discussions.len();
            for discussion in discussions {
                for note in discussion.notes {
                    if note.system {
                        continue;
                    }
                    let commit_oid = note
                        .position
                        .as_ref()
                        .and_then(|position| position.head_sha.as_deref())
                        .and_then(non_empty)
                        .or_else(|| note.commit_id.as_deref().and_then(non_empty))
                        .or_else(|| {
                            note.created_at
                                .as_ref()
                                .and_then(|created_at| version_head_at(versions, created_at))
                        });
                    records.push(PullRequestReviewRecord {
                        author: non_empty(&note.author.username),
                        submitted_at: note.created_at,
                        commit_oid,
                    });
                }
            }
            if received < 100 {
                break;
            }
        }
        Ok(records)
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
            requested.min(100).to_string(),
        ];
        // `PersonallyRequested` is not in GitLab's scope cycle.
        if matches!(
            query.scope,
            PullRequestListScope::ReviewRequested | PullRequestListScope::PersonallyRequested
        ) {
            args.push("--reviewer=@me".to_string());
        }
        // GitLab caps per_page at 100, so larger picker windows need multiple
        // API pages, including one extra row to determine whether more exist.
        let per_page = requested.min(100);
        let mut rows = Vec::new();
        for page in 1..=requested.div_ceil(per_page) {
            let mut page_args = args.clone();
            if page > 1 {
                page_args.extend(["--page".to_string(), page.to_string()]);
            }
            let output = self.run_glab(page_args, &query.repository.host)?;
            let page_rows: Vec<GlabMrSummary> = serde_json::from_str(&output)?;
            let received = page_rows.len();
            rows.extend(page_rows);
            if received < per_page {
                break;
            }
        }
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
        let project = self.api_project(&repository);
        let endpoint = format!("projects/{project}/merge_requests/{}", target.number);
        let output = self.run_api(&repository, endpoint)?;
        let mr: GlabMrDetails = serde_json::from_str(&output)?;
        mr.into_details(&repository)
    }

    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<Vec<FilePatch>> {
        let project = self.api_project(&pr.repository);
        let mut patches = Vec::new();
        for page in 1..=100 {
            let endpoint = format!(
                "projects/{project}/merge_requests/{}/diffs?per_page=100&page={page}",
                pr.number
            );
            let output = self.run_api(&pr.repository, endpoint)?;
            let rows: Vec<GlabDiff> = serde_json::from_str(&output)?;
            let received = rows.len();
            patches.extend(rows.into_iter().map(GlabDiff::into_file_patch));
            if received < 100 {
                return Ok(patches);
            }
        }
        Err(TuicrError::Forge(
            "GitLab merge request diff exceeded 10000 files".into(),
        ))
    }

    fn local_checkout_path(&self) -> Option<PathBuf> {
        self.local_checkout.clone()
    }

    fn list_pull_request_commits(&self, pr: &PullRequestDetails) -> Result<Vec<PullRequestCommit>> {
        let project = self.api_project(&pr.repository);
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
        // GitLab lists MR commits newest-first; the trait contract is
        // oldest-first.
        commits.reverse();
        Ok(commits)
    }

    fn list_pull_request_review_metadata(
        &self,
        pr: &PullRequestDetails,
    ) -> Result<PullRequestReviewMetadata> {
        let viewer_login = self.current_username(&pr.repository).unwrap_or_default();
        let versions = self.list_mr_versions(pr).unwrap_or_default();
        let mut reviews = self
            .list_mr_approval_records(pr, &versions)
            .unwrap_or_default();
        reviews.extend(
            self.list_mr_discussion_review_records(pr, &versions)
                .unwrap_or_default(),
        );
        Ok(PullRequestReviewMetadata {
            viewer_login,
            reviews,
        })
    }

    fn get_pull_request_commit_range_diff(
        &self,
        _pr: &PullRequestDetails,
        start_sha: &str,
        end_sha: &str,
    ) -> Result<Vec<FilePatch>> {
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
        let project = self.api_project(&pr.repository);
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
                    .filter_map(|d| d.into_review_thread(&pr.head_sha)),
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
        let drafts = self.list_draft_notes(pr)?;
        draft_notes_to_review(drafts, &mut all, None, &pr.url);
        Ok(all)
    }

    fn list_review_summaries(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewSummary>> {
        let drafts = self.list_draft_notes(pr)?;
        if drafts.is_empty() {
            return Ok(Vec::new());
        }
        let author = self.current_username(&pr.repository).ok().flatten();
        Ok(draft_notes_to_review(
            drafts,
            &mut Vec::new(),
            author.as_deref(),
            &pr.url,
        ))
    }

    fn fetch_file_lines(&self, request: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
        if request.start_line == 0 || request.start_line > request.end_line {
            return Ok(Vec::new());
        }
        let (start_line, end_line) = (request.start_line, request.end_line);
        let content = self.fetch_file_content(request)?;
        Ok(slice_context_lines(&content, start_line, end_line))
    }

    /// Local blob when the checkout has the PR's SHA, REST otherwise. The PR's
    /// exact SHAs may or may not be present locally; we silently fall back.
    fn fetch_file_content(&self, request: ForgeFileLinesRequest) -> Result<String> {
        match self
            .local_checkout
            .as_deref()
            .and_then(|root| read_blob(root, request.sha(), request.path.as_path()))
        {
            Some(content) => Ok(content),
            None => self.fetch_file_via_api(&request),
        }
    }

    fn file_line_count(&self, request: ForgeFileLinesRequest) -> Result<u32> {
        let content = self.fetch_file_content(request)?;
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

        let project = self.api_project(&pr.repository);
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
            let new_path = forge_path(&comment.path);
            // GitLab positions need both old_path and new_path. Renamed files
            // set comment.old_path to the base-side path; otherwise both
            // sides share the display path.
            let old_path = comment
                .old_path
                .as_ref()
                .map(|p| forge_path(p))
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
            // instead of collapsing to the end line. Without anchors the
            // endpoints' line codes cannot be built, so fall back to a
            // single-line comment on the end line rather than fail the submit.
            if let Some(anchors) = comment.range_anchors {
                position["line_range"] = serde_json::json!({
                    "start": gl_range_endpoint(&new_path, anchors.start),
                    "end": gl_range_endpoint(&new_path, anchors.end),
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
                let subfolder = self
                    .install_subfolder(&pr.repository.host)
                    .map(|sub| format!("/{sub}"))
                    .unwrap_or_default();
                let project_encoded = self.api_project(&pr.repository);
                let endpoint_label = if is_draft {
                    "draft_notes"
                } else {
                    "discussions"
                };
                let url = format!(
                    "https://{host}{subfolder}/api/v4/projects/{project_encoded}/merge_requests/{}/{endpoint_label}",
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
            let full_path = self.project_path(&pr.repository);
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
        let project = self.api_project(&request.repository);
        let encoded_path = gl_encode_file_path(&forge_path(&request.path));
        let endpoint = format!(
            "projects/{}/repository/files/{}/raw?ref={}",
            project,
            encoded_path,
            percent_encode_component(request.sha()),
        );
        let mut args = vec!["api".to_string()];
        args.extend(Self::api_hostname_args(&request.repository));
        args.push(endpoint);
        self.run_glab(args, &request.repository.host)
    }
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

/// How far host recognition may go to decide a remote is GitLab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostLookup {
    /// Consult `glab`'s configured host and `~/.ssh/config` when the hostname
    /// alone is inconclusive.
    WithConfig,
    /// Recognize GitLab by hostname only — no subprocess, no file read.
    HostnameOnly,
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
    parse_gitlab_remote(remote_url, HostLookup::WithConfig)
}

/// Like [`parse_gitlab_remote_url`], but recognizes GitLab by hostname alone:
/// no `glab config get host` subprocess and no `~/.ssh/config` read.
///
/// For callers on hot paths — [`crate::slug::resolve_owner_repo`] runs on every
/// session save — where a process spawn per call is not affordable. The cost is
/// coverage: a self-hosted instance on a custom domain like `git.example.com`
/// is not recognized, so it keeps falling back to the caller's generic parser.
pub fn parse_gitlab_remote_url_by_hostname(remote_url: &str) -> Option<ForgeRepository> {
    parse_gitlab_remote(remote_url, HostLookup::HostnameOnly)
}

fn parse_gitlab_remote(remote_url: &str, lookup: HostLookup) -> Option<ForgeRepository> {
    let trimmed = trim_url_suffix(remote_url.trim());
    if trimmed.is_empty() {
        return None;
    }

    if let Some((host, path)) = parse_scp_like_remote(trimmed) {
        if is_gitlab_host(host, lookup) {
            return gitlab_repository_from_path(host, path);
        }
        if lookup == HostLookup::HostnameOnly {
            return None;
        }
        // The remote may name an `~/.ssh/config` alias rather than the real
        // host, so fall back to what that alias resolves to.
        let resolved = resolve_ssh_hostname(host);
        if !is_gitlab_host(&resolved, lookup) {
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
    let host = strip_port(host);
    if !is_gitlab_host(host, lookup) {
        return None;
    }
    gitlab_repository_from_path(host, path)
}

/// Returns `true` when `host` looks like a GitLab instance: either its name
/// contains "gitlab" (covers `gitlab.com` and most public deployments) or —
/// under [`HostLookup::WithConfig`] — matches the default host `glab` is
/// configured against (covers self-hosted instances with custom domains like
/// `git.example.com`).
fn is_gitlab_host(host: &str, lookup: HostLookup) -> bool {
    if host.contains("gitlab") {
        return true;
    }
    if lookup == HostLookup::HostnameOnly {
        return false;
    }

    glab_default_host().is_some_and(|known| known.eq_ignore_ascii_case(host))
}

/// Read the default host configured in `glab` via `glab config get host`.
/// Used to recognize self-hosted GitLab instances whose domain doesn't
/// contain the substring "gitlab". Returns `None` when `glab` is missing,
/// the command fails, or no host is configured.
///
/// Only the hostname is returned. The value is often written as a URL
/// (`https://git.example.com:8443/gitlab`), and comparing it whole — as this
/// first did — meant a self-hosted instance on a neutral domain was never
/// claimed as GitLab at all. The path it may carry is deliberately ignored:
/// `glab` prefixes its API URLs from the `subfolder` key alone, so inferring an
/// install subfolder from this value would break requests that work today.
fn glab_default_host() -> Option<String> {
    let output = run_command_output("glab", None, ["config", "get", "host"]).ok()?;
    instance::parse_configured_hostname(&output)
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

/// Strip a trailing `:<port>` from a host, e.g. `example.com:2222` ->
/// `example.com`. `ssh://` remotes commonly carry a non-default SSH port
/// (self-hosted instances, GitLab's SSH-over-443 setup); that port is
/// meaningless for the HTTPS API host used to build `--repo` arguments, and
/// left in place it turns into a broken URL (wrong port, TLS failure).
pub(super) fn strip_port(host: &str) -> &str {
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

/// `glab` builds its API base URL from its own `subfolder` setting, so an
/// instance served from a relative URL that `glab` does not know about 404s
/// every call and tuicr cannot fix it from this side. Say so once, on the
/// error, rather than probing for it up front.
fn subfolder_hint(host: &str) -> String {
    format!(
        "\nIf GitLab is installed under a subfolder, tell `glab` about it: `glab config set subfolder <path> --host {host}`."
    )
}

fn looks_like_not_found(stderr: &str) -> bool {
    let lowered = stderr.to_ascii_lowercase();
    lowered.contains("404") || lowered.contains("not found")
}

/// `suggest_subfolder` appends [`subfolder_hint`] to a not-found failure. Only
/// callers that know the host has no configured subfolder pass `true`.
fn map_glab_error(error: GlabCommandError, host: &str, suggest_subfolder: bool) -> TuicrError {
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
            let hint = if suggest_subfolder && looks_like_not_found(&stderr) {
                subfolder_hint(host)
            } else {
                String::new()
            };
            let detail = if stderr.is_empty() {
                status
                    .map(|code| format!("glab exited with status {code}"))
                    .unwrap_or_else(|| "glab command failed".to_string())
            } else {
                stderr
            };
            TuicrError::Forge(format!("GitLab command failed: {detail}{hint}"))
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
/// The position schema allows an endpoint only `line_code` and `type`; adding
/// the line number rejects the whole note with `position => ["must be a valid
/// json schema"]`. `type` names the side a *changed* line belongs to and must
/// be omitted for context lines, which belong to both.
fn gl_range_endpoint(path: &str, anchor: DiffAnchor) -> serde_json::Value {
    let mut endpoint = serde_json::json!({
        "line_code": gl_line_code(path, anchor.old_line, anchor.new_line),
    });
    let line_type = match anchor.origin {
        LineOrigin::Addition => Some("new"),
        LineOrigin::Deletion => Some("old"),
        LineOrigin::Context => None,
    };
    if let Some(line_type) = line_type {
        endpoint["type"] = serde_json::Value::String(line_type.to_string());
    }
    endpoint
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
    map_glab_error(error, host, false)
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

/// Per-user location of the `TUICR_GLAB_DEBUG` log.
fn glab_debug_log_path() -> Option<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "tuicr")?;
    Some(dirs.cache_dir().join("glab-debug.log"))
}

/// Append `msg` to the debug log when `TUICR_GLAB_DEBUG` is set.
/// Uses a file so the TUI doesn't swallow the output.
fn glab_debug_log(msg: &str) {
    if let Some(path) = glab_debug_log_path() {
        let _ = append_private(&path, msg);
    }
}

/// Append to `path`, creating it `0600` and refusing to follow a symlink: the
/// log holds full API responses and comment bodies.
fn append_private(path: &std::path::Path, msg: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // A log that predates this check may be world-readable or foreign.
        let meta = file.metadata()?;
        // SAFETY: geteuid has no preconditions and cannot fail.
        if !meta.is_file() || meta.mode() & 0o077 != 0 || meta.uid() != unsafe { libc::geteuid() } {
            return Err(std::io::Error::other(format!(
                "{} is not an owner-only regular file",
                path.display()
            )));
        }
    }
    file.write_all(msg.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::forge::remote_comments::RemoteReviewState;

    #[cfg(unix)]
    #[test]
    fn append_private_creates_an_owner_only_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        append_private(&path, "a\n").unwrap();
        append_private(&path, "b\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\nb\n");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn append_private_does_not_follow_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let path = dir.path().join("log");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(append_private(&path, "secret").is_err());
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn append_private_refuses_an_existing_group_readable_log() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        std::fs::write(&path, "old\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(append_private(&path, "secret").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");
    }
    use crate::forge::submit::{DiffAnchor, GhSide, InlineComment, RangeAnchors};
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

    /// Model GitLab's server-side cap even when glab requests a larger page.
    struct CappedListRunner {
        total: usize,
    }

    impl GlabCommandRunner for CappedListRunner {
        fn run(&self, args: &[String]) -> GlabCommandResult<String> {
            let value = |flag: &str| {
                args.windows(2)
                    .find(|pair| pair[0] == flag)
                    .map(|pair| pair[1].parse::<usize>().unwrap())
            };
            let per_page = value("--per-page").unwrap().min(100);
            let page = value("--page").unwrap_or(1);
            let start = ((page - 1) * per_page).min(self.total);
            let rows = (start..(start + per_page).min(self.total))
                .map(|index| serde_json::json!({"iid": index + 1}))
                .collect::<Vec<_>>();
            Ok(serde_json::to_string(&rows).unwrap())
        }
    }

    #[test]
    fn list_pull_requests_paginates_beyond_gitlab_cap() {
        for (total, already_loaded, page_size) in [
            (150, 90, 30),
            (150, 120, 30),
            (100, 90, 30),
            (251, 0, 250),
            (251, 250, 30),
            (0, 0, 30),
        ] {
            let backend = GitLabGlabBackend::with_runner(None, CappedListRunner { total });
            let result = backend
                .list_pull_requests(PullRequestListQuery {
                    repository: ForgeRepository::gitlab("gitlab.com", "owner", "repo"),
                    already_loaded,
                    page_size,
                    scope: PullRequestListScope::Open,
                })
                .unwrap();
            let end = (already_loaded + page_size).min(total);
            assert_eq!(
                result
                    .pull_requests
                    .iter()
                    .map(|mr| mr.number)
                    .collect::<Vec<_>>(),
                (already_loaded + 1..=end)
                    .map(|n| n as u64)
                    .collect::<Vec<_>>(),
                "total={total}, already_loaded={already_loaded}",
            );
            assert_eq!(result.total_loaded, end);
            assert_eq!(result.has_more, total > end);
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

    #[test]
    fn get_pull_request_uses_version_compatible_api_endpoint() {
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let runner = RecordingRunner::new_with_responses(vec![
            r#"{
                "iid": 42,
                "title": "Test MR",
                "web_url": "https://gitlab.com/owner/repo/-/merge_requests/42",
                "state": "opened",
                "source_branch": "feature",
                "target_branch": "main",
                "diff_refs": {
                    "base_sha": "basesha1",
                    "head_sha": "headsha1",
                    "start_sha": "startsha1"
                }
            }"#
            .to_string(),
        ]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);

        let details = backend
            .get_pull_request(PullRequestTarget::number(42, "42"))
            .unwrap();

        assert_eq!(details.number, 42);
        assert_eq!(details.head_sha, "headsha1");
        let calls = backend.runner.calls.borrow();
        assert_eq!(
            calls[0].0,
            vec!["api", "projects/owner%2Frepo/merge_requests/42"]
        );
        assert!(
            !calls[0].0.iter().any(|arg| arg == "--output"),
            "glab 1.36 does not support `mr view --output`"
        );
    }

    /// A relative-URL install's subfolder lands in `owner` (it is
    /// indistinguishable from a top-level group in a clone URL), and `glab`
    /// prefixes it onto the API URL itself — so it must not also appear in the
    /// project id.
    fn subfolder_repo() -> ForgeRepository {
        ForgeRepository::gitlab("git.example.com", "gitlab/group", "project")
    }

    #[test]
    fn should_strip_install_subfolder_from_api_project_id() {
        let repo = subfolder_repo();
        let runner = RecordingRunner::new_with_responses(vec![
            r#"{
                "iid": 42,
                "title": "Test MR",
                "web_url": "https://git.example.com/gitlab/group/project/-/merge_requests/42",
                "state": "opened",
                "source_branch": "feature",
                "target_branch": "main",
                "diff_refs": {
                    "base_sha": "basesha1",
                    "head_sha": "headsha1",
                    "start_sha": "startsha1"
                }
            }"#
            .to_string(),
        ]);
        let backend = GitLabGlabBackend::with_runner_and_subfolder(
            Some(repo),
            runner,
            "git.example.com",
            Some("gitlab"),
        );

        backend
            .get_pull_request(PullRequestTarget::number(42, "42"))
            .unwrap();

        let calls = backend.runner.calls.borrow();
        assert_eq!(
            calls[0].0,
            vec![
                "api",
                "--hostname",
                "git.example.com",
                "projects/group%2Fproject/merge_requests/42",
            ]
        );
    }

    /// Regression test for the 404 in #567: this is the call that failed.
    #[test]
    fn should_strip_install_subfolder_when_listing_review_threads() {
        let repo = subfolder_repo();
        let pr = make_pr_details(repo.clone());
        let runner = RecordingRunner::new_with_responses(vec!["[]".to_string(), "[]".to_string()]);
        let backend = GitLabGlabBackend::with_runner_and_subfolder(
            Some(repo),
            runner,
            "git.example.com",
            Some("gitlab"),
        );

        backend.list_review_threads(&pr).unwrap();

        let calls = backend.runner.calls.borrow();
        assert!(
            calls[0].0.iter().any(|arg| arg
                == "projects/group%2Fproject/merge_requests/42/discussions?per_page=100&page=1"),
            "expected a subfolder-free project id, got {:?}",
            calls[0].0
        );
    }

    #[test]
    fn should_keep_project_id_when_no_subfolder_is_configured() {
        let repo = subfolder_repo();
        let pr = make_pr_details(repo.clone());
        let runner = RecordingRunner::new_with_responses(vec!["[]".to_string(), "[]".to_string()]);
        // `glab` is unaware of any subfolder, so it prefixes nothing and the
        // whole namespace is the project id — exactly as before this fix.
        let backend = GitLabGlabBackend::with_runner_and_subfolder(
            Some(repo),
            runner,
            "git.example.com",
            None,
        );

        backend.list_review_threads(&pr).unwrap();

        let calls = backend.runner.calls.borrow();
        assert!(
            calls[0].0.iter().any(|arg| arg
                .starts_with("projects/gitlab%2Fgroup%2Fproject/merge_requests/42/discussions")),
            "expected the namespace untouched, got {:?}",
            calls[0].0
        );
    }

    #[test]
    fn should_strip_install_subfolder_from_the_unencoded_graphql_project_path() {
        let repo = subfolder_repo();
        let runner = RecordingRunner::new_with_responses(Vec::new());
        let backend = GitLabGlabBackend::with_runner_and_subfolder(
            Some(repo.clone()),
            runner,
            "git.example.com",
            Some("gitlab"),
        );

        // What the `mergeRequestRequestChanges` mutation sends as `projectPath`.
        assert_eq!(backend.project_path(&repo), "group/project");
        assert_eq!(backend.api_project(&repo), "group%2Fproject");
    }

    #[test]
    fn should_not_look_up_a_subfolder_for_gitlab_com() {
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let runner = RecordingRunner::new_with_responses(Vec::new());
        let backend = GitLabGlabBackend::with_runner(Some(repo.clone()), runner);

        assert_eq!(backend.project_path(&repo), "owner/repo");
        assert!(
            backend.runner.calls.borrow().is_empty(),
            "gitlab.com cannot be under a subfolder, so it must cost no lookup"
        );
    }

    fn not_found_error() -> GlabCommandError {
        GlabCommandError::Failed {
            status: Some(1),
            stderr: "HTTP 404: 404 Project Not Found".to_string(),
        }
    }

    #[test]
    fn should_hint_at_subfolder_config_on_a_404() {
        let message = map_glab_error(not_found_error(), "git.example.com", true).to_string();
        assert!(
            message.contains("glab config set subfolder <path> --host git.example.com"),
            "expected a subfolder hint, got {message}"
        );
    }

    #[test]
    fn should_not_hint_at_subfolder_config_when_one_is_configured() {
        let message = map_glab_error(not_found_error(), "git.example.com", false).to_string();
        assert!(
            !message.contains("subfolder"),
            "expected no subfolder hint, got {message}"
        );
    }

    #[test]
    fn should_not_hint_at_subfolder_config_for_an_unrelated_failure() {
        let error = GlabCommandError::Failed {
            status: Some(1),
            stderr: "HTTP 500: something broke".to_string(),
        };
        let message = map_glab_error(error, "git.example.com", true).to_string();
        assert!(
            !message.contains("subfolder"),
            "expected no subfolder hint, got {message}"
        );
    }

    #[test]
    fn should_return_commits_oldest_first() {
        // given — GitLab lists MR commits newest-first
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let runner = RecordingRunner::new_with_responses(vec![
            r#"[
              {"id":"64ad53963568726a9ed23fd96b6d73203176775d","short_id":"64ad5396","title":"fixup!","author_name":"Radek","committed_date":"2026-08-27T13:49:53Z"},
              {"id":"e52022cd3bb22302b9775c7281ad27bddcf04f03","short_id":"e52022cd","title":"Edit README.md","author_name":"Radek","committed_date":"2026-08-27T13:48:53Z"}
            ]"#
                .to_string(),
        ]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);

        // when
        let commits = backend.list_pull_request_commits(&pr).unwrap();

        // then — the trait contract is chronological
        let summaries: Vec<&str> = commits.iter().map(|c| c.summary.as_str()).collect();
        assert_eq!(summaries, vec!["Edit README.md", "fixup!"]);
    }

    #[test]
    fn should_list_pull_request_review_metadata_for_gitlab() {
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let runner = RecordingRunner::new_with_responses(vec![
            r#"{ "username": "ronen-hoffer", "name": "Ronen" }"#.to_string(),
            r#"[
                {
                    "head_commit_sha": "ccccccc3333333333333333333333333333cccc",
                    "created_at": "2026-06-03T09:00:00Z"
                },
                {
                    "head_commit_sha": "bbbbbbb2222222222222222222222222222bbbb",
                    "created_at": "2026-06-02T09:00:00Z"
                }
            ]"#
            .to_string(),
            r#"{
                "approved_by": [
                    {
                        "user": { "username": "ronen-hoffer", "name": "Ronen" },
                        "approved_at": "2026-06-02T12:00:00Z"
                    }
                ]
            }"#
            .to_string(),
            r#"[
                {
                    "id": "disc-1",
                    "individual_note": false,
                    "notes": [
                        {
                            "id": 100,
                            "type": "DiffNote",
                            "body": "left a comment",
                            "author": { "username": "alice", "name": "Alice" },
                            "created_at": "2026-06-01T10:00:00Z",
                            "system": false,
                            "position": {
                                "position_type": "text",
                                "head_sha": "aaaaaaa1111111111111111111111111111aaaa",
                                "new_path": "src/lib.rs",
                                "new_line": 12
                            }
                        },
                        {
                            "id": 101,
                            "type": "DiffNote",
                            "body": "my latest comment",
                            "author": { "username": "ronen-hoffer", "name": "Ronen" },
                            "created_at": "2026-06-03T10:00:00Z",
                            "system": false,
                            "commit_id": "ccccccc3333333333333333333333333333cccc",
                            "position": {
                                "position_type": "text",
                                "head_sha": "ccccccc3333333333333333333333333333cccc",
                                "new_path": "src/lib.rs",
                                "new_line": 18
                            }
                        }
                    ]
                }
            ]"#
            .to_string(),
        ]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);

        let metadata = backend.list_pull_request_review_metadata(&pr).unwrap();

        assert_eq!(metadata.viewer_login.as_deref(), Some("ronen-hoffer"));
        assert_eq!(metadata.reviews.len(), 3);
        assert_eq!(
            metadata.reviews[0].commit_oid.as_deref(),
            Some("bbbbbbb2222222222222222222222222222bbbb")
        );
        assert_eq!(metadata.reviews[2].author.as_deref(), Some("ronen-hoffer"));
        assert_eq!(
            metadata.reviews[2].commit_oid.as_deref(),
            Some("ccccccc3333333333333333333333333333cccc")
        );

        let calls = backend.runner.calls.borrow();
        assert!(calls.iter().any(|(args, _)| args == &["api", "user"]));
        assert!(calls.iter().any(|(args, _)| {
            args.iter()
                .any(|arg| arg.contains("/versions?per_page=100&page=1"))
        }));
        assert!(
            calls
                .iter()
                .any(|(args, _)| args.iter().any(|arg| arg.contains("/approvals")))
        );
        assert!(calls.iter().any(|(args, _)| {
            args.iter()
                .any(|arg| arg.contains("/discussions?per_page=100&page=1"))
        }));
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
    fn should_fetch_structured_merge_request_diffs_instead_of_injecting_headers() {
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let runner = RecordingRunner::new_with_responses(vec![
            r#"[{
              "old_path":"日本語 b/and file.sql",
              "new_path":"日本語 b/and file.sql",
              "diff":"@@ -1,2 +1 @@\n--- count rows\n SELECT 1;\n"
            }]"#
            .to_string(),
        ]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);

        let patches = backend.get_pull_request_diff(&pr).unwrap();

        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].new_path.as_deref(),
            Some(Path::new("日本語 b/and file.sql"))
        );
        let calls = backend.runner.calls.borrow();
        assert!(
            calls[0]
                .0
                .iter()
                .any(|arg| arg.contains("/merge_requests/42/diffs?"))
        );
        assert!(!calls[0].0.iter().any(|arg| arg == "mr"));
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
            range_anchors: None,
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
            range_anchors: None,
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
            range_anchors: Some(RangeAnchors {
                start: DiffAnchor {
                    old_line: 20,
                    new_line: 25,
                    origin: LineOrigin::Addition,
                },
                end: DiffAnchor {
                    old_line: 20,
                    new_line: 30,
                    origin: LineOrigin::Addition,
                },
            }),
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
        // Endpoints carry the line code built from both counters plus `type`.
        // A line number here makes GitLab reject the position outright.
        assert_eq!(
            position["line_range"]["start"],
            serde_json::json!({
                "type": "new",
                "line_code": super::gl_line_code("src/lib.rs", 20, 25),
            })
        );
        assert_eq!(
            position["line_range"]["end"],
            serde_json::json!({
                "type": "new",
                "line_code": super::gl_line_code("src/lib.rs", 20, 30),
            })
        );
    }

    #[test]
    fn should_omit_endpoint_type_for_context_line_in_range() {
        // A context line belongs to both sides, so declaring a `type` for it
        // makes GitLab reject the position as an invalid schema.
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/lib.rs".into(),
            line: 84,
            side: GhSide::Right,
            counterpart_line: Some(70),
            start_line: Some(78),
            start_side: Some(GhSide::Right),
            range_anchors: Some(RangeAnchors {
                start: DiffAnchor {
                    old_line: 64,
                    new_line: 78,
                    origin: LineOrigin::Addition,
                },
                end: DiffAnchor {
                    old_line: 70,
                    new_line: 84,
                    origin: LineOrigin::Context,
                },
            }),
            old_path: None,
            body: "range ending on context".to_string(),
            comment_id: "c-range-ctx".to_string(),
        };
        let response = r#"{"id":"disc-range-ctx","individual_note":false}"#.to_string();
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
            position["line_range"]["end"],
            serde_json::json!({ "line_code": super::gl_line_code("src/lib.rs", 70, 84) }),
            "context endpoints carry only a line code"
        );
        // A range ending on a context line must name both sides in the position.
        assert_eq!(position["old_line"], serde_json::Value::Number(70.into()));
        assert_eq!(position["new_line"], serde_json::Value::Number(84.into()));
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
            range_anchors: None,
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
            range_anchors: None,
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
            range_anchors: None,
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
            range_anchors: None,
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
            range_anchors: None,
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
            range_anchors: None,
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
            range_anchors: None,
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

    /// A GitLab that keeps POSTed draft notes private: they come back only
    /// from `/draft_notes`, never from `/discussions`.
    #[derive(Default)]
    struct DraftStoreRunner {
        drafts: std::sync::Mutex<Vec<serde_json::Value>>,
    }

    impl GlabCommandRunner for DraftStoreRunner {
        fn run(&self, args: &[String]) -> GlabCommandResult<String> {
            let endpoint = args
                .iter()
                .find(|a| a.starts_with("projects/") || *a == "user");
            let body = match endpoint.map(String::as_str) {
                Some("user") => r#"{"username":"me"}"#.to_string(),
                Some(e) if e.contains("/draft_notes") => {
                    serde_json::to_string(&*self.drafts.lock().unwrap()).unwrap()
                }
                _ => "[]".to_string(),
            };
            Ok(body)
        }

        fn run_with_stdin(&self, _args: &[String], stdin: &str) -> GlabCommandResult<String> {
            let mut drafts = self.drafts.lock().unwrap();
            let mut note: serde_json::Value = serde_json::from_str(stdin).unwrap();
            note["id"] = serde_json::json!(drafts.len() + 1);
            drafts.push(note);
            Ok(serde_json::json!({"id": drafts.len()}).to_string())
        }
    }

    #[test]
    fn should_read_submitted_drafts_back_from_draft_notes() {
        // `:submit draft` posts to /draft_notes; the post-submit refetch has to
        // find them again or `prune_locked_comments` deletes the only copy.
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/lib.rs".into(),
            line: 15,
            side: GhSide::Right,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            range_anchors: None,
            old_path: None,
            body: "inline draft".to_string(),
            comment_id: "c1".to_string(),
        };
        let backend = GitLabGlabBackend::with_runner(Some(repo), DraftStoreRunner::default());
        backend
            .create_review(
                &pr,
                CreateReviewRequest {
                    event: crate::forge::submit::SubmitEvent::Draft,
                    commit_id: "headsha1",
                    body: "overall draft",
                    comments: &[inline],
                },
            )
            .unwrap();

        let threads = backend.list_review_threads(&pr).unwrap();
        let summaries = backend.list_review_summaries(&pr).unwrap();

        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].path, "src/lib.rs");
        assert_eq!(threads[0].line, Some(15));
        assert!(threads[0].comments[0].is_draft);
        assert_eq!(threads[0].comments[0].body, "inline draft");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].body, "overall draft");
        assert_eq!(summaries[0].state, RemoteReviewState::Pending);
        assert_eq!(summaries[0].author.as_deref(), Some("me"));
    }

    /// Serves `/draft_notes` in pages of the requested `per_page`.
    struct PagedDraftRunner {
        total: usize,
    }

    impl GlabCommandRunner for PagedDraftRunner {
        fn run(&self, args: &[String]) -> GlabCommandResult<String> {
            let endpoint = args
                .iter()
                .find(|a| a.starts_with("projects/") || *a == "user")
                .unwrap();
            if endpoint == "user" {
                return Ok(r#"{"username":"me"}"#.to_string());
            }
            if !endpoint.contains("/draft_notes?") {
                return Ok("[]".to_string());
            }
            let page: usize = endpoint.rsplit("page=").next().unwrap().parse().unwrap();
            let rows: Vec<_> = ((page - 1) * 100..(page * 100).min(self.total))
                .map(
                    |i| serde_json::json!({"id": i + 1, "note": format!("n{i}"), "position": null}),
                )
                .collect();
            Ok(serde_json::to_string(&rows).unwrap())
        }

        fn run_with_stdin(&self, _args: &[String], _stdin: &str) -> GlabCommandResult<String> {
            unreachable!()
        }
    }

    #[test]
    fn should_read_every_page_of_draft_notes() {
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        // Exactly two full pages, then an empty third: a reader that stops
        // after a full page, or after page one, loses drafts.
        let backend = GitLabGlabBackend::with_runner(Some(repo), PagedDraftRunner { total: 250 });
        assert_eq!(backend.list_review_summaries(&pr).unwrap().len(), 250);
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let backend = GitLabGlabBackend::with_runner(Some(repo), PagedDraftRunner { total: 200 });
        assert_eq!(backend.list_review_summaries(&pr).unwrap().len(), 200);
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
            range_anchors: None,
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
    fn should_strip_ssh_port_from_self_hosted_ssh_scheme_remote() {
        // `ssh://` remotes carrying a non-default SSH port (self-hosted
        // instances behind a custom port) must not leak that port into the
        // ForgeRepository host; it's meaningless for the HTTPS API and
        // breaks `--repo` URL construction (wrong port, TLS failure).
        let repo = parse_gitlab_remote_url(
            "ssh://git@gitlab.example.com:2222/engineering/platform/widget-service.git",
        )
        .unwrap();
        assert_eq!(repo.host, "gitlab.example.com");
        assert_eq!(repo.owner, "engineering/platform");
        assert_eq!(repo.name, "widget-service");
        assert_eq!(
            GitLabGlabBackend::<SystemGlabRunner>::repo_arg(&repo),
            "https://gitlab.example.com/engineering/platform/widget-service"
        );
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
    fn repo_arg_keeps_the_install_subfolder_in_the_browse_url() {
        // `owner` carries a relative-URL install's subfolder on purpose, and
        // `glab` strips its configured subfolder when it parses a `--repo`
        // URL, so this full clone-style URL resolves correctly. Only the API
        // project id has to drop it.
        let repo = ForgeRepository::gitlab("git.example.com", "gitlab/group", "project");
        assert_eq!(
            GitLabGlabBackend::<SystemGlabRunner>::repo_arg(&repo),
            "https://git.example.com/gitlab/group/project"
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

    fn file_request(
        path: &str,
        side: crate::forge::traits::ForgeFileSide,
    ) -> ForgeFileLinesRequest {
        ForgeFileLinesRequest {
            repository: ForgeRepository::gitlab("gitlab.com", "owner", "repo"),
            base_sha: "basesha".to_string(),
            head_sha: "headsha".to_string(),
            path: PathBuf::from(path),
            status: crate::model::FileStatus::Modified,
            side,
            start_line: 0,
            end_line: 0,
        }
    }

    #[test]
    fn fetch_file_content_falls_back_to_the_raw_endpoint_at_the_requested_side() {
        use crate::forge::traits::ForgeFileSide;
        // given — no local checkout
        let runner =
            RecordingRunner::new_with_responses(vec!["a\tb\n".to_string(), "old\n".to_string()]);
        let backend = GitLabGlabBackend::with_runner(None, runner);
        // when
        let head = backend
            .fetch_file_content(file_request("src/a.rs", ForgeFileSide::Head))
            .unwrap();
        let base = backend
            .fetch_file_content(file_request("src/a.rs", ForgeFileSide::Base))
            .unwrap();
        // then — exact bytes, and the ref follows the side
        assert_eq!(head, "a\tb\n");
        assert_eq!(base, "old\n");
        let calls = backend.runner.calls.borrow();
        assert_eq!(
            calls[0].0.last().unwrap(),
            "projects/owner%2Frepo/repository/files/src%2Fa.rs/raw?ref=headsha"
        );
        assert!(calls[1].0.last().unwrap().ends_with("/raw?ref=basesha"));
    }

    #[test]
    fn fetch_file_content_encodes_percent_and_url_syntax_in_the_path() {
        use crate::forge::traits::ForgeFileSide;
        // given — a literal `%2F` must not be read back as `/`
        let runner = RecordingRunner::new_with_responses(vec![String::new()]);
        let backend = GitLabGlabBackend::with_runner(None, runner);
        // when
        backend
            .fetch_file_content(file_request("a%2Fb#c?d e.rs", ForgeFileSide::Head))
            .unwrap();
        // then
        let calls = backend.runner.calls.borrow();
        assert!(
            calls[0]
                .0
                .last()
                .unwrap()
                .contains("/files/a%252Fb%23c%3Fd%20e.rs/raw"),
            "{:?}",
            calls[0].0
        );
    }

    #[cfg(unix)]
    #[test]
    fn should_keep_unix_backslashes_in_fetch_paths_and_comment_anchors() {
        use crate::forge::traits::ForgeFileSide;
        // given
        let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
        let pr = make_pr_details(repo.clone());
        let inline = InlineComment {
            path: "src/a\\b.rs".into(),
            line: 1,
            side: GhSide::Right,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            range_anchors: None,
            old_path: Some("old/c\\d.rs".into()),
            body: "x".to_string(),
            comment_id: "c1".to_string(),
        };
        let runner = RecordingRunner::new_with_responses(vec![
            r#"{"id":"d","individual_note":false}"#.to_string(),
            String::new(),
        ]);
        let backend = GitLabGlabBackend::with_runner(Some(repo), runner);
        // when
        backend
            .create_review(
                &pr,
                CreateReviewRequest {
                    event: crate::forge::submit::SubmitEvent::Comment,
                    commit_id: "headsha1",
                    body: "",
                    comments: &[inline],
                },
            )
            .unwrap();
        backend
            .fetch_file_content(file_request("src/a\\b.rs", ForgeFileSide::Head))
            .unwrap();
        // then
        let calls = backend.runner.calls.borrow();
        let body: serde_json::Value = serde_json::from_str(calls[0].1.as_ref().unwrap()).unwrap();
        assert_eq!(body["position"]["new_path"], "src/a\\b.rs");
        assert_eq!(body["position"]["old_path"], "old/c\\d.rs");
        assert!(calls[1].0.last().unwrap().contains("src%2Fa%5Cb.rs"));
    }
}
